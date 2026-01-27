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
    /// Connects to Postgres and attempts to acquire leadership.
    /// Returns the backend and optionally the leader info if this node became leader.
    /// The node is always registered in the nodes table regardless of leadership outcome.
    pub async fn connect(
        config: &PostgresConfig,
        bind_addr: SocketAddr,
    ) -> Result<(Self, Option<SequencerLeader>)> {
        // Compute node address for registration
        let node_address = node_address(bind_addr)?;

        let backend = Self::connect_internal(config, node_address).await?;

        let maybe_leader = backend
            .try_update_leader_and_register_node(LEADER_TIMEOUT)
            .await?;

        Ok((backend, maybe_leader))
    }

    /// Connects to Postgres and registers this node without attempting leader election.
    /// Used by Replica nodes that need to be visible in the cluster but should never
    /// become leader.
    pub async fn connect_as_replica(
        config: &PostgresConfig,
        bind_addr: SocketAddr,
    ) -> Result<Self> {
        let node_address = node_address(bind_addr)?;
        let backend = Self::connect_internal(config, node_address).await?;
        backend.upsert_node_registration_max_timestamp().await?;
        Ok(backend)
    }

    async fn connect_internal(config: &PostgresConfig, node_address: String) -> Result<Self> {
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

    /// Attempts to acquire or refresh leadership and register the node in the nodes table atomically.
    ///
    /// Returns `Some(leader)` if leadership was acquired (this node became leader, refreshed its
    /// leadership, or took over from a timed-out leader). Returns `None` if another node is
    /// the active leader and hasn't timed out yet. The node is always registered regardless.
    pub(crate) async fn try_update_leader_and_register_node(
        &self,
        leader_timeout: Duration,
    ) -> anyhow::Result<Option<SequencerLeader>> {
        run_with_retries!(
            &self.backoff_policy,
            self.try_update_leader_and_register_node_in_tx(leader_timeout),
            "postgres_db_backend_try_update_leader_and_register_node"
        )
    }

    async fn try_update_leader_and_register_node_in_tx(
        &self,
        leader_timeout: Duration,
    ) -> anyhow::Result<Option<SequencerLeader>> {
        let mut tx: sqlx::Transaction<'_, Postgres> = self.pool.begin().await?;
        let result = self
            .try_update_leader_inner(&mut tx, leader_timeout)
            .await?;
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

    /// Registers this node in the nodes table with the maximum possible timestamp.
    /// Used by Replica nodes that need to be visible in the cluster but don't participate in leader election.
    /// The infinity timestamp ensures that the replica nodes won't be pruned from the nodes table.
    async fn upsert_node_registration_max_timestamp(&self) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO nodes (node_id, address, last_updated)
             VALUES ($1, $2, 'infinity'::TIMESTAMPTZ)
             ON CONFLICT (node_id) DO UPDATE
             SET address = EXCLUDED.address,
                 last_updated = 'infinity'::TIMESTAMPTZ",
        )
        .bind(&self.node_id)
        .bind(&self.node_address)
        .execute(&self.pool)
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

#[cfg(test)]
mod tests {
    use super::*;
    use sov_full_node_configs::sequencer::NodeStartingRole;
    use sov_modules_api::VisibleSlotNumber;
    use sov_test_utils::postgres::{
        config_from_postgres_container, create_postgres_container, CreatePostgresError,
    };
    use std::num::NonZero;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sequencer_leader_election() {
        let postgres = create_postgres_container().await;
        let postgres = match postgres {
            Ok(pg) => pg,
            Err(CreatePostgresError::DockerNotSupported) => return,
            Err(CreatePostgresError::DockerError(e)) => {
                panic!("Failed to create Postgres container: {e}");
            }
        };

        let db_1 = &mut DB::new(&postgres, String::from("node_id_1"), NodeStartingRole::Leader).await;
        let db_2 = &mut DB::new(&postgres, String::from("node_id_2"), NodeStartingRole::Replica).await;

        {
            // Updating the same node_id should change the last updated time in the db.
            let leader_1 = db_1.maybe_update_leader().await.unwrap();
            let updated_leader_1 = db_1.maybe_update_leader().await.unwrap();

            assert_eq!(leader_1.node_id, db_1.node_id);
            assert_eq!(leader_1.node_id, updated_leader_1.node_id);
            assert!(leader_1.last_updated < updated_leader_1.last_updated);

            // Updating a different node id shouldn't change anything as the time delta is too big.
            let leader_2 = db_2.maybe_update_leader().await;
            assert!(leader_2.is_none());

            let leader_node_id = db_2.get_sequencer_leader().await.unwrap().unwrap();
            assert_eq!(updated_leader_1.node_id, leader_node_id);
        }

        {
            db_2.override_leader_timeout(Duration::ZERO);
            // Now we should be able to update db as the leader_timeout is zero.
            let leader_2 = db_2.maybe_update_leader().await.unwrap();
            assert_eq!(leader_2.node_id, db_2.node_id);
        }

        {
            db_1.override_leader_timeout(Duration::from_millis(100));
            let leader_1 = db_1.maybe_update_leader().await;
            assert!(leader_1.is_none());

            // Wait for more than 100ms and update the leader.
            tokio::time::sleep(Duration::from_millis(200)).await;
            let leader_1 = db_1.maybe_update_leader().await.unwrap();
            assert_eq!(leader_1.node_id, db_1.node_id);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_db_operations_leader() {
        let postgres = create_postgres_container().await;
        let postgres = match postgres {
            Ok(pg) => pg,
            Err(CreatePostgresError::DockerNotSupported) => return,
            Err(CreatePostgresError::DockerError(e)) => {
                panic!("Failed to create Postgres container: {e}");
            }
        };

        let db = &mut DB::new(&postgres, String::from("node_id_1"), NodeStartingRole::Leader).await;
        db.maybe_update_leader().await.unwrap();

        let sequence_number = 1;
        let batch_to_store = batch_to_store(sequence_number);

        db.as_mut()
            .begin_rollup_block(batch_to_store)
            .await
            .unwrap();

        db.as_mut()
            .add_tx(
                sequence_number,
                1,
                FullyBakedTx::new(vec![1, 2, 3]),
                TxHash::new([1; 32]),
            )
            .await
            .unwrap();

        db.as_mut()
            .batch_add_txs(
                sequence_number,
                2,
                &[(FullyBakedTx::new(vec![4, 5, 6]), TxHash::new([1; 32]))],
            )
            .await
            .unwrap();

        db.as_mut()
            .add_proof_blob(sequence_number, 3, Arc::new([1, 2, 3]))
            .await
            .unwrap();

        db.as_mut().end_rollup_block(batch_to_store).await.unwrap();

        let data = db.as_mut().current_data().await.unwrap();
        assert!(!data.is_empty());

        db.as_mut().prune(2).await.unwrap();
        let data = db.as_mut().current_data().await.unwrap();
        assert!(data.is_empty());

        db.as_mut().prune(2).await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_db_operations_replica() {
        let postgres = create_postgres_container().await;
        let postgres = match postgres {
            Ok(pg) => pg,
            Err(CreatePostgresError::DockerNotSupported) => return,
            Err(CreatePostgresError::DockerError(e)) => {
                panic!("Failed to create Postgres container: {e}");
            }
        };
        let db_leader = &mut DB::new(&postgres, String::from("node_id_1"), NodeStartingRole::Leader).await;
        let db_replica =
            &mut DB::new(&postgres, String::from("node_id_2"), NodeStartingRole::Replica).await;

        let sequence_number = 1;
        let batch_to_store = batch_to_store(sequence_number);

        db_leader.maybe_update_leader().await.unwrap();

        let err = db_replica
            .as_mut()
            .begin_rollup_block(batch_to_store)
            .await
            .unwrap_err();

        assert_err(err, &db_replica.node_id, &FailedOperation::BeginBlock);

        let err = db_replica
            .as_mut()
            .add_tx(
                sequence_number,
                1,
                FullyBakedTx::new(vec![1, 2, 3]),
                TxHash::new([1; 32]),
            )
            .await
            .unwrap_err();

        assert_err(err, &db_replica.node_id, &FailedOperation::AddTx);

        let err = db_replica
            .as_mut()
            .batch_add_txs(
                sequence_number,
                2,
                &[(FullyBakedTx::new(vec![4, 5, 6]), TxHash::new([1; 32]))],
            )
            .await
            .unwrap_err();

        assert_err(err, &db_replica.node_id, &FailedOperation::BatchAddTxs);

        let err = db_replica
            .as_mut()
            .add_proof_blob(sequence_number, 3, Arc::new([1, 2, 3]))
            .await
            .unwrap_err();

        assert_err(err, &db_replica.node_id, &FailedOperation::AddProof);

        let err = db_replica
            .as_mut()
            .end_rollup_block(batch_to_store)
            .await
            .unwrap_err();

        assert_err(err, &db_replica.node_id, &FailedOperation::EndBlock);

        let err = db_replica.as_mut().prune(2).await.unwrap_err();
        assert_err(
            err,
            &db_replica.node_id,
            &FailedOperation::Prune {
                db_leader: Some(db_leader.node_id.clone()),
            },
        );

        let err = db_replica.as_mut().current_data().await.unwrap_err();
        assert_err(
            err,
            &db_replica.node_id,
            &FailedOperation::CurrentData {
                db_leader: Some(db_leader.node_id.clone()),
            },
        );
    }

    fn assert_err(err: DbError, expected_node_id: &String, expected_operation: &FailedOperation) {
        match &err {
            DbError::ReplicaDisallowed {
                self_node_id,
                operation,
            } => {
                assert_eq!(self_node_id, expected_node_id);
                assert_eq!(operation, expected_operation);
            }
            DbError::Database(err) => unreachable!("DbError::Database not allowed in test {err:?}"),
        }
    }

    struct DB {
        backend: PostgresBackend,
        node_id: String,
        leader_timeout: Duration,
    }

    impl AsRef<PostgresBackend> for DB {
        fn as_ref(&self) -> &PostgresBackend {
            &self.backend
        }
    }

    impl AsMut<PostgresBackend> for DB {
        fn as_mut(&mut self) -> &mut PostgresBackend {
            &mut self.backend
        }
    }

    impl DB {
        async fn new(
            postgres: &sov_test_utils::postgres::ContainerAsync<sov_test_utils::postgres::Postgres>,
            node_id: String,
            node_role: NodeStartingRole,
        ) -> Self {
            let leader_timeout = Duration::from_millis(100_000);
            let postgres_config =
                config_from_postgres_container(postgres, node_id.clone(), node_role)
                    .await
                    .unwrap();
            let backend =
                PostgresBackend::connect_internal(&postgres_config, format!("{node_id}_address"))
                    .await
                    .unwrap();

            Self {
                backend,
                node_id,
                leader_timeout,
            }
        }

        fn override_leader_timeout(&mut self, leader_timeout: Duration) {
            self.leader_timeout = leader_timeout;
        }

        async fn maybe_update_leader(&self) -> Option<SequencerLeader> {
            self.backend
                .try_update_leader_and_register_node(self.leader_timeout)
                .await
                .unwrap()
        }

        pub(crate) async fn get_sequencer_leader(&self) -> Result<Option<String>, sqlx::Error> {
            let mut tx = self.backend.pool.begin().await?;
            let res = self.backend.get_sequencer_leader_inner(&mut tx).await?;
            tx.commit().await?;
            Ok(res)
        }
    }

    fn batch_to_store(sequence_number: SequenceNumber) -> BatchToStore {
        BatchToStore {
            blob_id: 1,
            sequence_number,
            visible_slot_number_after_increase: VisibleSlotNumber::new_dangerous(1),
            visible_slots_to_advance: NonZero::new(1).unwrap(),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_nodes_table_notifications() {
        let postgres = create_postgres_container().await;
        let postgres = match postgres {
            Ok(pg) => pg,
            Err(CreatePostgresError::DockerNotSupported) => return,
            Err(CreatePostgresError::DockerError(e)) => {
                panic!("Failed to create Postgres container: {e}");
            }
        };

        let db = DB::new(&postgres, String::from("node_1"), NodeStartingRole::Leader).await;

        let mut listener = sqlx::postgres::PgListener::connect_with(&db.backend.pool)
            .await
            .unwrap();

        listener.listen("nodes_changes").await.unwrap();

        // Test INSERT notification via try_update_leader_and_register_node
        db.maybe_update_leader().await.unwrap();

        let notification = tokio::time::timeout(Duration::from_secs(5), listener.recv())
            .await
            .expect("Timed out waiting for INSERT notification")
            .unwrap();

        assert_eq!(notification.channel(), "nodes_changes");
        let parts: Vec<&str> = notification.payload().split(',').collect();
        assert_eq!(parts, vec!["node_1", "node_1_address", "INSERT"]);

        // Test UPDATE notification via try_update_leader_and_register_node
        db.maybe_update_leader().await.unwrap();

        let notification = tokio::time::timeout(Duration::from_secs(5), listener.recv())
            .await
            .expect("Timed out waiting for UPDATE notification")
            .unwrap();

        assert_eq!(notification.channel(), "nodes_changes");
        let parts: Vec<&str> = notification.payload().split(',').collect();
        assert_eq!(parts, vec!["node_1", "node_1_address", "UPDATE"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_leader_acquired_at() {
        let postgres = create_postgres_container().await;
        let postgres = match postgres {
            Ok(pg) => pg,
            Err(CreatePostgresError::DockerNotSupported) => return,
            Err(CreatePostgresError::DockerError(e)) => {
                panic!("Failed to create Postgres container: {e}");
            }
        };

        let db_1 = &mut DB::new(&postgres, String::from("node_1"), NodeStartingRole::Leader).await;
        let db_2 = &mut DB::new(&postgres, String::from("node_2"), NodeStartingRole::Replica).await;

        // Node 1 becomes leader
        db_1.maybe_update_leader().await.unwrap();

        let (initial_leader_acquired_at,): (OffsetDateTime,) =
            sqlx::query_as("SELECT leader_acquired_at FROM sequencer_leader WHERE singleton = 1")
                .fetch_one(&db_1.backend.pool)
                .await
                .unwrap();

        // Small delay to ensure time difference
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Same node refreshes leadership (heartbeat)
        db_1.maybe_update_leader().await.unwrap();

        let (after_refresh_leader_acquired_at,): (OffsetDateTime,) =
            sqlx::query_as("SELECT leader_acquired_at FROM sequencer_leader WHERE singleton = 1")
                .fetch_one(&db_1.backend.pool)
                .await
                .unwrap();

        assert_eq!(
            initial_leader_acquired_at, after_refresh_leader_acquired_at,
            "leader_acquired_at should not change when same node refreshes leadership"
        );

        // Different node takes over leadership after timeout
        db_2.override_leader_timeout(Duration::ZERO);
        db_2.maybe_update_leader().await.unwrap();

        let (after_takeover_leader_acquired_at,): (OffsetDateTime,) =
            sqlx::query_as("SELECT leader_acquired_at FROM sequencer_leader WHERE singleton = 1")
                .fetch_one(&db_2.backend.pool)
                .await
                .unwrap();

        assert!(
            after_takeover_leader_acquired_at > initial_leader_acquired_at,
            "leader_acquired_at should be updated when a different node takes over leadership"
        );
    }
}
