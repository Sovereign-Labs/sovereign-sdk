#[cfg(test)]
mod tests;

use crate::preferred::db::FailedOperation;
use anyhow::{anyhow, Result};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use super::{DbBackend, ReadBlob, SnapshotData, StoredBlob};
use crate::preferred::db::DbError;
use crate::preferred::db::{BatchToStore, DbReadOutcome, InProgressBatch};
use anyhow::Context;
use axum::async_trait;
use backon::{BackoffBuilder, ExponentialBuilder};
use sov_blob_sender::BlobInternalId;
use sov_blob_storage::SequenceNumber;
use sov_full_node_configs::sequencer::PostgresConfig;
use sov_modules_api::{FullyBakedTx, TxHash};
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::FromRow;
use sqlx::{PgConnection, Postgres};
use time::OffsetDateTime;

/// The leader timeout used for leader election.
pub(crate) const LEADER_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, FromRow, PartialEq)]
pub(crate) struct SequencerLeader {
    pub(crate) node_id: String,
    pub(crate) last_updated: OffsetDateTime,
}

pub struct PostgresBackend {
    pool: PgPool,
    backoff_policy: ExponentialBuilder,
    node_id: String,
    pub(crate) node_address: String,
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
    /// Connects to Postgres and competes for leadership.                                                                                                                                                                                                    
    ///                                                                                                                                                                                                                                                      
    /// Returns the backend and `Some(leader)` if this node became leader,                                                                                                                                                                                   
    /// or `None` if another node is the active leader.                                                                                                                                                                                                      
    /// The node is always registered in the cluster regardless of leadership outcome.
    pub async fn connect_as_maybe_leader(
        config: &PostgresConfig,
        bind_addr: SocketAddr,
    ) -> Result<(Self, Option<SequencerLeader>)> {
        let backend = Self::connect(config, bind_addr).await?;
        let maybe_leader = backend.heartbeat(Some(LEADER_TIMEOUT)).await?;
        Ok((backend, maybe_leader))
    }

    /// Connects to Postgres as a replica without competing for leadership.                                                                                                                                                                                  
    ///                                                                                                                                                                                                                                                      
    /// Registers the node in the cluster but never attempts to become leader
    pub async fn connect_as_replica(
        config: &PostgresConfig,
        bind_addr: SocketAddr,
    ) -> Result<Self> {
        let backend = Self::connect(config, bind_addr).await?;
        let _maybe_leader = backend.heartbeat(None).await?;
        Ok(backend)
    }

    /// // Connects to Postgres db.
    pub async fn connect(config: &PostgresConfig, bind_addr: SocketAddr) -> Result<Self> {
        // Compute node address for registration
        let node_address = node_address(bind_addr)?;
        let connection_string = &config.postgres_connection_string;
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
            node_id: config.node_id.clone(),
            node_address,
        })
    }

    async fn read_blob<Inner: From<InProgressBatch>>(
        &self,
        sequence_number: SequenceNumber,
        stored_blob: StoredBlob,
        connection: &mut PgConnection,
        with_retries: bool,
    ) -> Result<ReadBlob<Inner>> {
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
                            anyhow!("Invalid database data for tx hash; check your database integrity: {err:?}")
                        })?))
                    })
                    .collect::<Result<Vec<_>>>()?;
                // Deserialize the full FullyBakedTx (including sequencing_data)
                let txs = txs
                    .into_iter()
                    .map(|data| borsh::from_slice::<FullyBakedTx>(&data))
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(ReadBlob::Batch(
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
            StoredBlob::Proof { data, blob_id } => Ok(ReadBlob::Proof {
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
    ) -> Result<Option<InProgressBatch>> {
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
            ReadBlob::Batch(batch) => Ok(Some(batch)),
            ReadBlob::Proof { .. } => panic!(
                "Expected a batch blob, but got a proof blob. This is a bug, please report it"
            ),
        }
    }
    /// Read all the current data as a single transaction. We have to attempt the whole transaction atomically,
    /// which is why this is wrapped in a helper function and any nested helpers have their retries disabled.
    async fn current_data_transaction(&self) -> Result<DbReadOutcome<SnapshotData>> {
        let mut tx = self.pool.begin().await?;
        let maybe_leader = self.get_sequencer_leader_inner(&mut tx).await?;

        if !self.is_leader(&maybe_leader) {
            return Ok(DbReadOutcome::AbortedBecauseReplica {
                db_leader: maybe_leader,
            });
        }

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

        Ok(DbReadOutcome::Success(SnapshotData {
            completed_blobs,
            in_progress_batch,
        }))
    }

    /// Sends a heartbeat to update this node's registration and optionally compete for leadership.
    ///
    /// This method always updates the node's entry in the `nodes` table with the current timestamp.
    ///
    /// If `leader_timeout` is `Some(duration)`, also attempts to acquire or refresh leadership.
    /// Returns `Some(leader)` if this node became or remains the leader.
    /// Returns `None` if another active leader exists or if leadership competition was skipped.
    pub(crate) async fn heartbeat(
        &self,
        leader_timeout: Option<Duration>,
    ) -> anyhow::Result<Option<SequencerLeader>> {
        run_with_retries!(
            &self.backoff_policy,
            self.heartbeat_in_tx(leader_timeout),
            "postgres_db_backend_heartbeat"
        )
    }

    async fn heartbeat_in_tx(
        &self,
        leader_timeout: Option<Duration>,
    ) -> anyhow::Result<Option<SequencerLeader>> {
        let mut tx: sqlx::Transaction<'_, Postgres> = self.pool.begin().await?;
        let result = if let Some(leader_timeout) = leader_timeout {
            self.try_update_leader_inner(&mut tx, leader_timeout)
                .await?
        } else {
            None
        };
        self.upsert_node_registration_inner(&mut tx).await?;
        tx.commit().await?;
        Ok(result)
    }

    async fn try_update_leader_inner(
        &self,
        conn: &mut PgConnection,
        leader_timeout: Duration,
    ) -> anyhow::Result<Option<SequencerLeader>> {
        let leader_timeout: i64 = leader_timeout
            .as_millis()
            .try_into()
            // It is ok to `expect` as leader_timeout should be much smaller than i64::MAX
            .expect("PostgresBackend error: leader_timeout is bigger than i64::MAX");

        let res = sqlx::query_as::<_, SequencerLeader>(
            "WITH ts AS (SELECT NOW() as current_time)
            INSERT INTO sequencer_leader (node_id, last_updated)
            SELECT $1, ts.current_time FROM ts
                ON CONFLICT (singleton) DO UPDATE
                    SET
                        node_id = EXCLUDED.node_id,
                        last_updated = EXCLUDED.last_updated,
                        leader_acquired_at = CASE
                            WHEN sequencer_leader.node_id != EXCLUDED.node_id THEN EXCLUDED.last_updated
                            ELSE sequencer_leader.leader_acquired_at
                        END
                    WHERE
                        sequencer_leader.node_id = EXCLUDED.node_id
                        OR sequencer_leader.last_updated < EXCLUDED.last_updated - ($2 * INTERVAL '1 millisecond')
                    RETURNING node_id, last_updated",
        )
        .bind(&self.node_id)
        .bind(leader_timeout)
        .fetch_optional(&mut *conn)
        .await?;

        Ok(res)
    }

    async fn upsert_node_registration_inner(&self, conn: &mut PgConnection) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO nodes (node_id, address, last_updated)
             VALUES ($1, $2, NOW())
             ON CONFLICT (node_id) DO UPDATE
             SET address = EXCLUDED.address,
                 last_updated = NOW()",
        )
        .bind(&self.node_id)
        .bind(&self.node_address)
        .execute(&mut *conn)
        .await?;
        Ok(())
    }

    async fn prune_inner(
        &self,
        prune_up_to_including: SequenceNumber,
    ) -> anyhow::Result<DbReadOutcome<()>> {
        let mut tx = self.pool.begin().await?;
        let prune_up_to_including: i64 = prune_up_to_including.saturating_add(1).try_into()?;

        let result = sqlx::query(
            "WITH blobs_deleted AS (
                    DELETE FROM proof_blobs
                    WHERE sequence_number <= $1
                    AND is_leader($2))
                DELETE FROM events
                    WHERE sequence_number <= $1 AND is_leader($2);",
        )
        .bind(prune_up_to_including)
        .bind(&self.node_id)
        .execute(&mut *tx)
        .await?;

        if result.rows_affected() == 0 {
            let maybe_leader = self.get_sequencer_leader_inner(&mut tx).await?;
            if !self.is_leader(&maybe_leader) {
                return Ok(DbReadOutcome::AbortedBecauseReplica {
                    db_leader: maybe_leader,
                });
            }
        }
        tx.commit().await?;
        Ok(DbReadOutcome::Success(()))
    }

    async fn get_sequencer_leader_inner(
        &self,
        connection: &mut PgConnection,
    ) -> Result<Option<String>, sqlx::Error> {
        let maybe_leader: Option<SequencerLeader> = sqlx::query_as::<_, SequencerLeader>(
            "SELECT node_id, last_updated
                FROM sequencer_leader
                WHERE singleton = 1",
        )
        .fetch_optional(connection)
        .await?;

        Ok(maybe_leader.map(|l| l.node_id))
    }

    fn is_leader(&self, maybe_leader_id: &Option<String>) -> bool {
        match maybe_leader_id {
            Some(leader_id) => leader_id == &self.node_id,
            None => false,
        }
    }
}

#[async_trait]
impl DbBackend for PostgresBackend {
    async fn begin_rollup_block(&mut self, batch_to_store: BatchToStore) -> Result<(), DbError> {
        let blob_data = borsh::to_vec(&StoredBlob::Batch {
            blob_id: batch_to_store.blob_id,
            visible_slot_number_after_increase: batch_to_store.visible_slot_number_after_increase,
            visible_slots_to_advance: batch_to_store.visible_slots_to_advance,
        })?;

        // Compound CTE statement to avoid multiple roundtrips
        let result = run_with_retries!(
            &self.backoff_policy,
            sqlx::query(
                "
            WITH batch_insert AS (
                INSERT INTO in_progress_batch (sequence_number, borsh_value)
                SELECT $1, $2
                WHERE is_leader($3)
            RETURNING sequence_number
            )
            INSERT INTO events (sequence_number, event_type, index_in_batch, hash, data)
            SELECT bi.sequence_number, 'batch_start', NULL, NULL, $2
            FROM batch_insert bi;
            "
            )
            .bind(i64::try_from(batch_to_store.sequence_number)?)
            .bind::<&[u8]>(blob_data.as_ref())
            .bind(&self.node_id)
            .execute(&self.pool),
            "postgres_db_backend_begin_rollup_block"
        )?;

        if result.rows_affected() == 0 {
            return Err(DbError::ReplicaDisallowed {
                self_node_id: self.node_id.clone(),
                operation: FailedOperation::BeginBlock,
            });
        }

        Ok(())
    }

    async fn batch_add_txs(
        &mut self,
        sequence_number: SequenceNumber,
        tx_idx_within_batch: u64,
        txs: &[(FullyBakedTx, TxHash)],
    ) -> anyhow::Result<(), DbError> {
        let start = i64::try_from(tx_idx_within_batch)?;
        let end = start + txs.len() as i64;
        let sequence_number = vec![i64::try_from(sequence_number)?; txs.len()];
        let event_types = vec!["transaction"; txs.len()];
        let tx_indexes = (start..end).collect::<Vec<_>>();
        let hashes = txs.iter().map(|(_, hash)| hash.0).collect::<Vec<_>>();
        // Serialize the full FullyBakedTx (including sequencing_data) to preserve metadata
        let txs = txs
            .iter()
            .map(|(tx, _)| borsh::to_vec(tx).unwrap())
            .collect::<Vec<_>>();
        let txs_refs: Vec<&[u8]> = txs.iter().map(|t| t.as_slice()).collect();

        let result = run_with_retries!(
            &self.backoff_policy,
            sqlx::query::<Postgres>(
                "INSERT INTO events (sequence_number, event_type, index_in_batch, hash, data)
                SELECT *
                    FROM UNNEST(
                    $1::bigint[],
                    $2::event_type[],
                    $3::bigint[],
                    $4::bytea[],
                    $5::bytea[]
                )
                WHERE is_leader($6);"
            )
            .bind(&sequence_number[..])
            .bind(&event_types[..])
            .bind(&tx_indexes[..])
            .bind(&hashes[..])
            .bind(&txs_refs[..])
            .bind(&self.node_id)
            .execute(&self.pool),
            "postgres_db_backend_add_tx"
        )?;

        if result.rows_affected() == 0 {
            return Err(DbError::ReplicaDisallowed {
                self_node_id: self.node_id.clone(),
                operation: FailedOperation::BatchAddTxs,
            });
        }

        Ok(())
    }

    async fn add_tx(
        &mut self,
        sequence_number: SequenceNumber,
        tx_index_within_batch: u64,
        tx: FullyBakedTx,
        hash: TxHash,
    ) -> anyhow::Result<(), DbError> {
        // Serialize the full FullyBakedTx (including sequencing_data) to preserve metadata
        let tx_serialized = borsh::to_vec(&tx)?;
        let result = run_with_retries!(
            &self.backoff_policy,
            sqlx::query::<Postgres>(
                "INSERT INTO events (sequence_number, event_type, index_in_batch, hash, data)
                    SELECT $1, 'transaction', $2, $3, $4
                WHERE is_leader($5);",
            )
            .bind(i64::try_from(sequence_number)?)
            .bind(i64::try_from(tx_index_within_batch)?)
            .bind::<&[u8]>(hash.as_ref())
            .bind(&tx_serialized)
            .bind(&self.node_id)
            .execute(&self.pool),
            "postgres_db_backend_add_tx"
        )?;

        if result.rows_affected() == 0 {
            return Err(DbError::ReplicaDisallowed {
                self_node_id: self.node_id.clone(),
                operation: FailedOperation::AddTx,
            });
        }

        Ok(())
    }

    async fn end_rollup_block(&mut self, cached: BatchToStore) -> Result<(), DbError> {
        let sequence_number = cached.sequence_number;
        let stored_blob: StoredBlob = cached.into();
        let blob_data = borsh::to_vec(&stored_blob)?;

        // Compound CTE statement to avoid multiple roundtrips
        let result = run_with_retries!(
            &self.backoff_policy,
            sqlx::query(
                "WITH batch_delete AS (
                    DELETE FROM in_progress_batch
                    WHERE is_leader($3)
                    RETURNING 1)
                INSERT INTO events (sequence_number, event_type, index_in_batch, hash, data)
                SELECT $1, 'batch_end', NULL, NULL, $2
                FROM batch_delete",
            )
            .bind(i64::try_from(sequence_number)?)
            .bind::<&[u8]>(blob_data.as_ref())
            .bind(&self.node_id)
            .execute(&self.pool),
            "postgres_db_backend_end_rollup_block"
        )?;

        if result.rows_affected() == 0 {
            return Err(DbError::ReplicaDisallowed {
                self_node_id: self.node_id.clone(),
                operation: FailedOperation::EndBlock,
            });
        }

        Ok(())
    }

    async fn prune(&mut self, up_to_including: SequenceNumber) -> Result<(), DbError> {
        let outcome = run_with_retries!(
            &self.backoff_policy,
            self.prune_inner(up_to_including),
            "postgres_db_backend_prune"
        )?;

        if let DbReadOutcome::AbortedBecauseReplica { db_leader } = outcome {
            return Err(DbError::ReplicaDisallowed {
                self_node_id: self.node_id.clone(),
                operation: FailedOperation::Prune { db_leader },
            });
        }

        Ok(())
    }
    async fn read_in_progress_batch(&self) -> anyhow::Result<Option<InProgressBatch>, DbError> {
        let mut tx = self.pool.begin().await?;
        let maybe_leader = self.get_sequencer_leader_inner(&mut tx).await?;

        if !self.is_leader(&maybe_leader) {
            return Err(DbError::ReplicaDisallowed {
                self_node_id: self.node_id.clone(),
                operation: FailedOperation::ReadBatch,
            });
        }

        let res = self
            .read_in_progress_batch_with_connection(&mut tx, true)
            .await;
        tx.commit().await?;

        Ok(res?)
    }

    async fn add_proof_blob(
        &mut self,
        sequence_number: SequenceNumber,
        blob_id: BlobInternalId,
        data: Arc<[u8]>,
    ) -> Result<(), DbError> {
        let blob_data = borsh::to_vec(&StoredBlob::Proof { data, blob_id })?;

        // Compound CTE statement to avoid multiple roundtrips
        let result = run_with_retries!(
            &self.backoff_policy,
            sqlx::query(
                "
                WITH blob_insert AS (
                    INSERT INTO proof_blobs (sequence_number, borsh_value)
                    SELECT $1, $2
                    WHERE is_leader($3)
                    RETURNING sequence_number
                )
                INSERT INTO events (sequence_number, event_type, index_in_batch, hash, data)
                SELECT bi.sequence_number, 'new_proof', NULL, NULL, NULL
                FROM blob_insert bi",
            )
            .bind(i64::try_from(sequence_number)?)
            .bind::<&[u8]>(blob_data.as_ref())
            .bind(&self.node_id)
            .execute(&self.pool),
            "postgres_db_backend_add_proof_blob"
        )?;

        if result.rows_affected() == 0 {
            return Err(DbError::ReplicaDisallowed {
                self_node_id: self.node_id.clone(),
                operation: FailedOperation::AddProof,
            });
        }

        Ok(())
    }

    async fn current_data(&self) -> anyhow::Result<SnapshotData, DbError> {
        let res = run_with_retries!(
            &self.backoff_policy,
            self.current_data_transaction(),
            "postgres_db_backend_current_data"
        )?;

        match res {
            DbReadOutcome::Success(data) => Ok(data),
            DbReadOutcome::AbortedBecauseReplica { db_leader } => {
                return Err(DbError::ReplicaDisallowed {
                    self_node_id: self.node_id.clone(),
                    operation: FailedOperation::CurrentData { db_leader },
                });
            }
        }
    }
}

/// Computes the node address from the local IP and bind port.
fn node_address(bind_addr: SocketAddr) -> Result<String> {
    let bind_port = bind_addr.port();
    let ip = bind_addr.ip();

    let effective_ip = if ip.is_unspecified() {
        get_local_ip(ip)?
    } else {
        ip
    };

    let effective_addr = SocketAddr::new(effective_ip, bind_port);
    Ok(effective_addr.to_string())
}

/// Gets the local IP address by creating a UDP socket and checking its local address.
fn get_local_ip(ip: IpAddr) -> Result<std::net::IpAddr> {
    // This is a classic networking trick to figure out your machine’s local IP address,
    // without actually sending any data.
    let addr = if ip.is_ipv6() {
        let socket = std::net::UdpSocket::bind("[::]:0").with_context(|| {
            format!("Failed to bind UDP socket for local IPv6 address discovery: {ip}")
        })?;
        socket
            .connect("[2001:4860:4860::8888]:80")
            .context("Failed to connect UDP socket for local IPv6 address discovery")?;
        socket
    } else {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0").with_context(|| {
            format!("Failed to bind UDP socket for local IPv4 address discovery: {ip}")
        })?;
        socket
            .connect("8.8.8.8:80")
            .context("Failed to connect UDP socket for local IPv4 address discovery.")?;

        socket
    }
    .local_addr()
    .context("Failed to retrieve local address from UDP socket.")?;

    Ok(addr.ip())
}
