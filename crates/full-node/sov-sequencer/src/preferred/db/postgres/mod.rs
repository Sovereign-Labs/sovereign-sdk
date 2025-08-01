use std::num::NonZero;
use std::sync::Arc;
use std::time::Duration;

use axum::async_trait;
use backon::{BackoffBuilder, ExponentialBuilder};
use sov_blob_sender::BlobInternalId;
use sov_blob_storage::SequenceNumber;
use sov_modules_api::{FullyBakedTx, TxHash};
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::{PgConnection, Postgres};

use super::{DbSnapshotData, PreferredSequencerDbBackend, PreferredSequencerReadBlob, StoredBlob};
use crate::preferred::db::{BatchToStore, InProgressBatch};

pub struct PostgresBackend {
    pool: PgPool,
    backoff_policy: ExponentialBuilder,
}

// We need a macro to get around lifetime issues with async functions. Otherwise, Rust complains about FnMut
// outliving the lifetime of the function.
macro_rules! run_with_retries {
    ($backoff_policy:expr, $fxn:expr, $method_name:expr) => {
        {
            let mut result = $fxn.await;
            if result.is_ok() {
                result
            } else {
                for iter in $backoff_policy.clone().build() {
                    tracing::warn!(
                        // Safety: We just checked that the result is an error, so we can unwrap.
                        method_name = %$method_name, error = %result.err().unwrap(), duration = ?iter,
                        "Error in Postgres backend. Retrying in specified duration."
                    );
                    tokio::time::sleep(iter).await;
                    result = $fxn.await;
                    if result.is_ok() {
                        break;
                    }
                }
                result
            }
        }
    };
}

impl PostgresBackend {
    pub async fn connect(connection_string: &str) -> anyhow::Result<Self> {
        // This backoff policy should usually terminate in a second.
        // Running the numbers... We do 8 retries, doubling the sleep each time that yields 256ms max delay and an average delay of ~50ms
        // So total runtime is ~400 ms of sleeping. If we also account for 50ms latency on each roundtrip, we get about 800ms total time
        let backoff_policy = ExponentialBuilder::default()
            .with_jitter()
            .with_min_delay(Duration::from_millis(2))
            .with_max_delay(Duration::from_millis(500))
            .with_factor(2.0)
            .with_max_times(8);

        let pool = run_with_retries!(
            &backoff_policy,
            PgPoolOptions::default().connect(connection_string),
            "postgres_db_backend_connect"
        )?;

        run_with_retries!(
            &backoff_policy,
            sqlx::migrate!("src/preferred/db/postgres/migrations").run(&pool),
            "postgres_db_backend_migrate"
        )?;

        Ok(Self {
            pool,
            backoff_policy,
        })
    }

    async fn read_blob<Inner: From<InProgressBatch>>(
        &self,
        sequence_number: SequenceNumber,
        stored_blob: StoredBlob,
        connection: &mut PgConnection,
        with_retries: bool,
    ) -> anyhow::Result<PreferredSequencerReadBlob<Inner>> {
        let backoff_policy = self.maybe_retry_policy(with_retries).await;
        match stored_blob {
            StoredBlob::Batch {
                visible_slot_number_after_increase,
                visible_slots_to_advance,
                blob_id,
            } => {
                let tx_rows: Vec<(Vec<u8>, Vec<u8>)> = run_with_retries!(
                        &backoff_policy,
                        sqlx::query_as::<Postgres, _>(
                        "SELECT hash, data FROM events WHERE sequence_number = $1 AND event_type = 'transaction' ORDER BY index_in_batch",
                    )
                    .bind(i64::try_from(sequence_number)?)
                    .fetch_all(&mut *connection),
                    "postgres_db_backend_read_blob_txs"
                )?;

                let (tx_hashes, txs): (Vec<_>, Vec<_>) = tx_rows.into_iter().unzip();
                let tx_hashes = tx_hashes
                    .into_iter()
                    .map(|bytes| {
                        Ok(TxHash::new(bytes.try_into().map_err(|err| {
                            anyhow::anyhow!("Invalid database data for tx hash; check your database integrity: {err:?}")
                        })?))
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?;
                let txs = txs.into_iter().map(FullyBakedTx::new).collect::<Vec<_>>();

                Ok(PreferredSequencerReadBlob::Batch(
                    InProgressBatch {
                        sequence_number,
                        visible_slot_number_after_increase,
                        visible_slots_to_advance,
                        txs,
                        tx_hashes,
                        blob_id,
                    }
                    .into(),
                ))
            }
            StoredBlob::Proof { data, blob_id } => Ok(PreferredSequencerReadBlob::Proof {
                sequence_number,
                blob_id,
                data,
            }),
        }
    }
    async fn maybe_retry_policy(&self, with_retries: bool) -> ExponentialBuilder {
        if with_retries {
            self.backoff_policy
        } else {
            ExponentialBuilder::default().with_max_times(0)
        }
    }

    async fn read_in_progress_batch_with_connection(
        &self,
        connection: &mut PgConnection,
        with_retries: bool,
    ) -> anyhow::Result<Option<InProgressBatch>> {
        let backoff_policy = self.maybe_retry_policy(with_retries).await;
        let Some((sequence_number, stored_blob_serialized)): Option<(i64, Vec<u8>)> = run_with_retries!(
            &backoff_policy,
            sqlx::query_as::<Postgres, _>(
                "SELECT sequence_number, borsh_value FROM in_progress_batch",
            )
            .fetch_optional(&mut *connection),
            "postgres_db_backend_read_in_progress_batch"
        )?
        else {
            return Ok(None);
        };

        let sequence_number = SequenceNumber::try_from(sequence_number)?;
        let stored_blob = borsh::from_slice(&stored_blob_serialized)?;

        match self
            .read_blob(sequence_number, stored_blob, connection, true)
            .await?
        {
            PreferredSequencerReadBlob::Batch(batch) => Ok(Some(batch)),
            PreferredSequencerReadBlob::Proof { .. } => panic!(
                "Expected a batch blob, but got a proof blob. This is a bug, please report it"
            ),
        }
    }
    /// Read all the current data as a single transaction. We have to attempt the whole transaction atomically,
    /// which is why this is wrapped in a helper function and any nested helpers have their retries disabled.
    async fn current_data_transaction(&self) -> anyhow::Result<DbSnapshotData> {
        let mut tx = self.pool.begin().await?;

        let latest_event_id: Option<u64> =
            sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(event_id) FROM events")
                .fetch_one(&mut *tx)
                .await?
                .map(|id| id as u64);

        let completed_blobs_metadata: Vec<(i64, Vec<u8>)> =
            sqlx::query_as::<Postgres, _>(
                "SELECT sequence_number, data FROM events WHERE event_type = 'batch_end' ORDER BY sequence_number",
            )
            .fetch_all(&mut *tx)
            .await?;

        // Fill out completed blobs with transaction data
        let mut completed_blobs = Vec::new();
        for (sequence_number, stored_blob_serialized) in completed_blobs_metadata {
            let sequence_number = SequenceNumber::try_from(sequence_number)?;
            let stored_blob = borsh::from_slice(&stored_blob_serialized)?;
            completed_blobs.push(
                self.read_blob(sequence_number, stored_blob, &mut tx, false)
                    .await?,
            );
        }

        let in_progress_batch = self
            .read_in_progress_batch_with_connection(&mut tx, false)
            .await?;

        tx.commit().await?;

        Ok(DbSnapshotData {
            completed_blobs,
            in_progress_batch,
            latest_event_id,
        })
    }
}

#[async_trait]
impl PreferredSequencerDbBackend for PostgresBackend {
    async fn begin_rollup_block(
        &mut self,
        sequence_number: SequenceNumber,
        blob_id: BlobInternalId,
        visible_slot_number_after_increase: sov_modules_api::VisibleSlotNumber,
        visible_slots_to_advance: NonZero<u8>,
    ) -> anyhow::Result<()> {
        let blob_data = borsh::to_vec(&StoredBlob::Batch {
            blob_id,
            visible_slot_number_after_increase,
            visible_slots_to_advance,
        })?;

        // Compound CTE statement to avoid multiple roundtrips
        run_with_retries!(
            &self.backoff_policy,
            sqlx::query(
                "WITH batch_insert AS (
                    INSERT INTO in_progress_batch (sequence_number, borsh_value) VALUES ($1, $2)
                )
             INSERT INTO events (sequence_number, event_type, index_in_batch, hash, data) 
             SELECT $1, 'batch_start', NULL, NULL, $2",
            )
            .bind(i64::try_from(sequence_number)?)
            .bind::<&[u8]>(blob_data.as_ref())
            .execute(&self.pool),
            "postgres_db_backend_begin_rollup_block"
        )?;

        Ok(())
    }

    async fn batch_add_txs(
        &mut self,
        sequence_number: SequenceNumber,
        tx_idx_within_batch: u64,
        txs: &[(FullyBakedTx, TxHash)],
    ) -> anyhow::Result<()> {
        let start = i64::try_from(tx_idx_within_batch)?;
        let end = start + txs.len() as i64;
        let sequence_number = vec![i64::try_from(sequence_number)?; txs.len()];
        let event_types = vec!["transaction"; txs.len()];
        let tx_indexes = (start..end).collect::<Vec<_>>();
        let hashes = txs.iter().map(|(_, hash)| hash.0).collect::<Vec<_>>();
        let txs = txs.iter().map(|(tx, _)| &tx.data).collect::<Vec<_>>();
        run_with_retries!(
            &self.backoff_policy,
            sqlx::query::<Postgres>(
                "INSERT INTO events (sequence_number, event_type, index_in_batch, hash, data)
                SELECT * FROM UNNEST($1::bigint[], $2::event_type[], $3::bigint[], $4::bytea[], $5::bytea[])"
            )
            .bind(&sequence_number[..])
            .bind(&event_types[..])
            .bind(&tx_indexes[..])
            .bind(&hashes[..])
            .bind(&txs[..])
            .execute(&self.pool),
            "postgres_db_backend_add_tx"
        )?;
        Ok(())
    }

    async fn add_tx(
        &mut self,
        sequence_number: SequenceNumber,
        tx_index_within_batch: u64,
        tx: FullyBakedTx,
        hash: TxHash,
    ) -> anyhow::Result<()> {
        run_with_retries!(
            &self.backoff_policy,
            sqlx::query::<Postgres>(
                "INSERT INTO events (sequence_number, event_type, index_in_batch, hash, data) VALUES ($1, 'transaction', $2, $3, $4)",
            )
            .bind(i64::try_from(sequence_number)?)
            .bind(i64::try_from(tx_index_within_batch)?)
            .bind::<&[u8]>(hash.as_ref())
            .bind(&tx.data)
            .execute(&self.pool),
            "postgres_db_backend_add_tx"
        )?;

        Ok(())
    }

    async fn end_rollup_block(&mut self, cached: BatchToStore) -> anyhow::Result<()> {
        let sequence_number = cached.sequence_number;
        let stored_blob: StoredBlob = cached.into();
        let blob_data = borsh::to_vec(&stored_blob)?;

        // Compound CTE statement to avoid multiple roundtrips
        let result = run_with_retries!(
            &self.backoff_policy,
            sqlx::query(
                "WITH batch_delete AS (
                    DELETE FROM in_progress_batch
                RETURNING 1
            )
            INSERT INTO events (sequence_number, event_type, index_in_batch, hash, data) 
            SELECT $1, 'batch_end', NULL, NULL, $2
            FROM batch_delete",
            )
            .bind(i64::try_from(sequence_number)?)
            .bind::<&[u8]>(blob_data.as_ref())
            .execute(&self.pool),
            "postgres_db_backend_end_rollup_block"
        )?;

        if result.rows_affected() == 0 {
            return Err(anyhow::anyhow!("No in-progress batch found to end"));
        }

        Ok(())
    }

    async fn prune(&mut self, up_to_including: SequenceNumber) -> anyhow::Result<()> {
        // Compound CTE statement to avoid multiple roundtrips
        run_with_retries!(
            &self.backoff_policy,
            sqlx::query(
                "WITH blobs_deleted AS (
                    DELETE FROM proof_blobs WHERE sequence_number <= $1
             )
             DELETE FROM events WHERE sequence_number <= $1",
            )
            .bind(i64::try_from(up_to_including)?)
            .execute(&self.pool),
            "postgres_db_backend_prune"
        )?;

        Ok(())
    }
    async fn read_in_progress_batch(&self) -> anyhow::Result<Option<InProgressBatch>> {
        let mut conn = self.pool.acquire().await?;
        self.read_in_progress_batch_with_connection(&mut conn, true)
            .await
    }

    async fn add_proof_blob(
        &mut self,
        sequence_number: SequenceNumber,
        blob_id: BlobInternalId,
        data: Arc<[u8]>,
    ) -> anyhow::Result<()> {
        let blob_data = borsh::to_vec(&StoredBlob::Proof { data, blob_id })?;

        // Compound CTE statement to avoid multiple roundtrips
        run_with_retries!(
            &self.backoff_policy,
            sqlx::query(
                "WITH blob_insert AS (
                    INSERT INTO proof_blobs (sequence_number, borsh_value) VALUES ($1, $2)
             )
             INSERT INTO events (sequence_number, event_type, index_in_batch, hash, data) 
             SELECT $1, 'new_proof', NULL, NULL, NULL",
            )
            .bind(i64::try_from(sequence_number)?)
            .bind::<&[u8]>(blob_data.as_ref())
            .execute(&self.pool),
            "postgres_db_backend_add_proof_blob"
        )?;

        Ok(())
    }

    async fn current_data(&self) -> anyhow::Result<DbSnapshotData> {
        run_with_retries!(
            &self.backoff_policy,
            self.current_data_transaction(),
            "postgres_db_backend_current_data"
        )
    }
}
