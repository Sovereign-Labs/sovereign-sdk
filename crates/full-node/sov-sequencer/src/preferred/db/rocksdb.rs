use std::num::NonZero;
use std::path::Path;
use std::sync::Arc;

use axum::async_trait;
use rockbound::{gen_rocksdb_options, SchemaBatch};
use sov_blob_sender::BlobInternalId;
use sov_blob_storage::SequenceNumber;
use sov_modules_api::{FullyBakedTx, TxHash, VisibleSlotNumber};

use super::{DbSnapshotData, PreferredSequencerDbBackend, PreferredSequencerReadBlob, StoredBlob};
use crate::preferred::db::{BatchToStore, InProgressBatch};

#[derive(Debug)]
pub struct RocksDbBackend {
    db: Arc<rockbound::DB>,
    // Overlapping range deletes are a RocksDB antipattern and can cause *massive* memory blowup on compaction/memtable flush.
    // We remember which ranges we've pruned so far to avoid overlapping deletes.
    //
    // https://github.com/facebook/rocksdb/wiki/DeleteRange-Implementation
    first_unpruned_sequence_number: SequenceNumber,
}

#[async_trait]
impl PreferredSequencerDbBackend for RocksDbBackend {
    #[tracing::instrument(skip_all, level = "trace")]
    async fn read_in_progress_batch(&self) -> anyhow::Result<Option<InProgressBatch>> {
        let Some((sequence_number, stored_blob)) =
            self.db.get_async::<tables::InProgressBatch>(&()).await?
        else {
            return Ok(None);
        };

        match self.read_blob(sequence_number, stored_blob).await? {
            PreferredSequencerReadBlob::Batch(batch) => {
                Ok(Some(batch))
            }
            _ => panic!("In-progress batch must be a batch but is a proof blob; this is a bug, please report it"),
        }
    }

    #[tracing::instrument(skip_all, level = "trace")]
    async fn begin_rollup_block(
        &mut self,
        sequence_number: SequenceNumber,
        blob_id: BlobInternalId,
        visible_slot_number_after_increase: VisibleSlotNumber,
        visible_slots_to_advance: NonZero<u8>,
    ) -> anyhow::Result<()> {
        self.db
            .put_async::<tables::InProgressBatch>(
                &(),
                &(
                    sequence_number,
                    StoredBlob::Batch {
                        visible_slot_number_after_increase,
                        visible_slots_to_advance,
                        blob_id,
                    },
                ),
            )
            .await?;

        Ok(())
    }

    #[tracing::instrument(skip_all, level = "trace")]
    async fn add_tx(
        &mut self,
        sequence_number: SequenceNumber,
        tx_idx_within_batch: u64,
        tx: FullyBakedTx,
        hash: TxHash,
    ) -> anyhow::Result<()> {
        self.db
            .put_async::<tables::BatchContents>(
                &(sequence_number, tx_idx_within_batch),
                &(hash, tx),
            )
            .await?;

        Ok(())
    }

    #[tracing::instrument(skip_all, level = "trace")]
    async fn end_rollup_block(&mut self, stored_batch: BatchToStore) -> anyhow::Result<()> {
        let sequence_number = stored_batch.sequence_number;
        let stored_blob: StoredBlob = stored_batch.into();
        let mut s = SchemaBatch::new();
        s.delete::<tables::InProgressBatch>(&())?;
        s.put::<tables::CompletedBlobs>(&sequence_number, &stored_blob)?;
        self.db.write_schemas_async(&s).await?;

        Ok(())
    }

    #[tracing::instrument(skip_all, level = "trace")]
    async fn prune(&mut self, prune_up_to_including: SequenceNumber) -> anyhow::Result<()> {
        // We first delete blob data, and only then batch contents. We'd rather have orphaned
        // data (no harm in that, it'll just get pruned eventually) than batches
        // incorrectly marked as empty.
        //
        // Alternatively, a cross-column-family atomic delete would also work.

        // Avoid overlapping range deletes.
        if prune_up_to_including < self.first_unpruned_sequence_number {
            // Warn if we skipped pruning because the sequence number went down. If it merely stayed the same, this is expected behavior so skip the warning.
            if prune_up_to_including != self.first_unpruned_sequence_number.checked_sub(1).expect("Sequence number underflow. This is unreachable because we've just checked that prune_up_to_including < self.first_unpruned_sequence_number") {
                tracing::warn!(
                    sequence_number = %prune_up_to_including,
                    "Skipping pruning of sequence number because it's already been pruned",
                );
            }
            return Ok(());
        }

        self.db.delete_range::<tables::CompletedBlobs>(
            &self.first_unpruned_sequence_number,
            // The upper bound is exclusive.
            &prune_up_to_including.saturating_add(1),
        )?;

        self.db.delete_range::<tables::BatchContents>(
            &(self.first_unpruned_sequence_number, u64::MIN),
            &(prune_up_to_including, u64::MAX),
        )?;
        self.first_unpruned_sequence_number = prune_up_to_including.checked_add(1).expect(
            "Sequence number overflow. This should be unreachable in the next few billion years",
        );

        Ok(())
    }

    #[tracing::instrument(skip_all, level = "trace")]
    async fn add_proof_blob(
        &mut self,
        sequence_number: SequenceNumber,
        blob_id: BlobInternalId,
        data: Arc<[u8]>,
    ) -> anyhow::Result<()> {
        self.db
            .put_async::<tables::CompletedBlobs>(
                &sequence_number,
                &StoredBlob::Proof { data, blob_id },
            )
            .await?;
        Ok(())
    }

    async fn current_data(&self) -> anyhow::Result<DbSnapshotData> {
        // RocksDB doesn't need atomicity, and doesn't track event_ids
        let completed_blobs = self.read_completed_blobs().await?;
        let in_progress_batch = self.read_in_progress_batch().await?;
        Ok(DbSnapshotData {
            completed_blobs,
            in_progress_batch,
            latest_event_id: None,
        })
    }
}

impl RocksDbBackend {
    const DB_NAME: &'static str = "preferred_sequencer";
    const TABLES: &'static [&'static str] = &[
        tables::CompletedBlobs::table_name(),
        tables::InProgressBatch::table_name(),
        tables::BatchContents::table_name(),
    ];

    /// Opens a new [`RocksDbBackend`] at the given path.
    pub async fn new(path: &Path) -> anyhow::Result<Self> {
        let db = Arc::new(rockbound::DB::open(
            path.join(Self::DB_NAME),
            Self::DB_NAME,
            Self::TABLES.iter().copied(),
            &gen_rocksdb_options(&Default::default(), false),
        )?);

        // There's an edge case where we might have an in-progress batch but no completed blobs. In that case,
        // we'll use zero instead of the lowest sequencer number - but a single overlapping range delete is no big deal
        // so we're fine with that.
        let mut iter = db.iter::<tables::CompletedBlobs>()?;
        iter.seek(&SequenceNumber::MIN)?;
        let first_unpruned_sequence_number = iter
            .next()
            .transpose()?
            .map(|item| item.key)
            .unwrap_or(SequenceNumber::MIN);
        // Explicitly drop the iterator to avoid a borrow checker error.
        // Rustc tries to drop it at the end of the scope, and then complains that `db`
        // is borrowed when we move it into `Self`.
        drop(iter);

        Ok(Self {
            db,
            first_unpruned_sequence_number,
        })
    }

    #[tracing::instrument(skip_all, level = "trace")]
    async fn read_completed_blobs(&self) -> anyhow::Result<Vec<PreferredSequencerReadBlob>> {
        let mut blobs = vec![];

        // Iteration might be slow, but getters are only called during
        // sequencer initialization so it's okay.
        for item_res in self.db.iter::<tables::CompletedBlobs>()? {
            let item = item_res?;
            let sequence_number = item.key;
            let stored_blob = item.value;

            blobs.push(self.read_blob(sequence_number, stored_blob).await?);
        }

        Ok(blobs)
    }

    #[cfg(test)]
    pub fn trigger_compaction(&self) {
        self.db
            .trigger_compaction::<tables::BatchContents>()
            .expect("Compaction failed");
    }

    async fn read_blob<Inner: From<InProgressBatch>>(
        &self,
        sequence_number: SequenceNumber,
        stored_blob: StoredBlob,
    ) -> anyhow::Result<PreferredSequencerReadBlob<Inner>> {
        Ok(match stored_blob {
            StoredBlob::Batch {
                visible_slot_number_after_increase,
                visible_slots_to_advance,
                blob_id,
            } => {
                let mut txs = vec![];
                let mut tx_hashes = vec![];

                // Iteration might be slow, but getters are only called during
                // sequencer initialization so it's okay.
                let mut iter = self.db.iter::<tables::BatchContents>()?;
                iter.seek(&(sequence_number, u64::MIN))?;

                for item_res in iter {
                    let item = item_res?;
                    if item.key.0 != sequence_number {
                        break;
                    }
                    let (tx_hash, tx) = item.value;
                    txs.push(tx);
                    tx_hashes.push(tx_hash);
                }

                PreferredSequencerReadBlob::Batch(
                    InProgressBatch {
                        sequence_number,
                        visible_slot_number_after_increase,
                        visible_slots_to_advance,
                        txs,
                        tx_hashes,
                        blob_id,
                    }
                    .into(),
                )
            }
            StoredBlob::Proof { data, blob_id } => PreferredSequencerReadBlob::Proof {
                sequence_number,
                data,
                blob_id,
            },
        })
    }
}

mod tables {
    use sov_db::{
        define_table_with_default_codec, define_table_with_seek_key_codec,
        define_table_without_codec, impl_borsh_value_codec,
    };

    use super::*;
    use crate::preferred::db::StoredBlob;

    define_table_with_seek_key_codec!(
        (CompletedBlobs) SequenceNumber => StoredBlob
    );

    define_table_with_default_codec!(
        (InProgressBatch) () => (SequenceNumber, StoredBlob)
    );

    define_table_with_seek_key_codec!(
        (BatchContents) (SequenceNumber, u64) => (TxHash, FullyBakedTx)
    );
}

#[cfg(test)]
mod tests {
    use sov_modules_api::HexString;
    use tempfile::TempDir;

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_overlapping_range_deletion_pathology() {
        run_rocksdb_test(2000).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn flaky_test_sequencer_rocksdb_db_performance_can_run_1k_batches_in_90_seconds() {
        let handle = tokio::task::spawn(run_rocksdb_test(10000));

        let _ = tokio::time::timeout(std::time::Duration::from_secs(90), handle)
            .await
            .expect("Creating 10000 batches should take less than 90 seconds; you may need to check your filesystem performance!");
    }

    async fn run_rocksdb_test(iters: u64) {
        let dir = TempDir::new().unwrap();
        let mut db = RocksDbBackend::new(dir.path()).await.unwrap();

        // Trigger `iters` batches of 10 txs. Ensure that performance stays reasonable
        for batch in 0u64..iters {
            db.begin_rollup_block(
                SequenceNumber::from(batch),
                BlobInternalId::from(batch),
                VisibleSlotNumber::new_dangerous(batch),
                NonZero::new(1).unwrap(),
            )
            .await
            .unwrap();

            let mut txs = vec![];
            let mut tx_hashes = vec![];
            for i in 0..10 {
                let tx = FullyBakedTx {
                    data: vec![i as u8; 200],
                };
                let tx_hash = HexString([i as u8; 32]);
                db.add_tx(SequenceNumber::from(batch), i, tx.clone(), tx_hash)
                    .await
                    .unwrap();
                txs.push(tx);
                tx_hashes.push(tx_hash);
            }
            let batch_to_store = BatchToStore {
                sequence_number: SequenceNumber::from(batch),
                visible_slot_number_after_increase: VisibleSlotNumber::new_dangerous(batch),
                visible_slots_to_advance: NonZero::new(1).unwrap(),
                blob_id: BlobInternalId::from(batch),
            };
            db.end_rollup_block(batch_to_store).await.unwrap();

            if batch > 1 {
                db.prune(SequenceNumber::from(batch)).await.unwrap();
            }
        }

        // This should trigger the pathology
        let task = tokio::task::spawn_blocking(move || {
            db.trigger_compaction(); // Should OOM or take forever
        });
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .expect("Compaction should take less than 1 second");
    }
}
