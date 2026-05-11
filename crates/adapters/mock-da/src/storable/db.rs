use std::str::FromStr;

use anyhow::{anyhow, Context};
use chrono::{DateTime, Utc};
use sov_rollup_interface::da::Time;
#[cfg(feature = "postgres")]
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
#[cfg(feature = "postgres")]
use sqlx::Postgres;
use sqlx::{ConnectOptions, Executor, QueryBuilder, Sqlite};

use crate::config::GENESIS_HEADER;
use crate::utils::hash_to_array;
use crate::{MockAddress, MockBlob, MockBlockHeader, MockHash};

pub(crate) const BATCH_NAMESPACE: &str = "batches";
pub(crate) const PROOF_NAMESPACE: &str = "proofs";
const FINALIZED_HEIGHT_ID: i32 = 1;

#[derive(Debug)]
pub(crate) enum DbPool {
    Sqlite(SqlitePool),
    #[cfg(feature = "postgres")]
    Postgres(PgPool),
}

pub(crate) enum DbTx<'a> {
    Sqlite(sqlx::Transaction<'a, Sqlite>),
    #[cfg(feature = "postgres")]
    Postgres(sqlx::Transaction<'a, Postgres>),
}

#[derive(Clone, Debug)]
pub(crate) struct BlobHashData {
    pub(crate) id: i32,
    pub(crate) hash: Vec<u8>,
    pub(crate) sender: Vec<u8>,
    pub(crate) namespace: String,
}

#[derive(Clone, Debug)]
pub(crate) struct DbBlob {
    pub(crate) hash: Vec<u8>,
    pub(crate) data: Vec<u8>,
    pub(crate) namespace: String,
    pub(crate) sender: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(crate) struct DbBlockHeader {
    pub(crate) height: i32,
    pub(crate) prev_hash: Vec<u8>,
    pub(crate) hash: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(crate) struct NewBlob {
    block_height: i32,
    hash: Vec<u8>,
    data: Vec<u8>,
    namespace: String,
    sender: Vec<u8>,
}

impl DbPool {
    pub(crate) async fn begin(&self) -> Result<DbTx<'_>, sqlx::Error> {
        match self {
            Self::Sqlite(pool) => Ok(DbTx::Sqlite(pool.begin().await?)),
            #[cfg(feature = "postgres")]
            Self::Postgres(pool) => Ok(DbTx::Postgres(pool.begin().await?)),
        }
    }
}

impl DbTx<'_> {
    pub(crate) async fn commit(self) -> Result<(), sqlx::Error> {
        match self {
            Self::Sqlite(tx) => tx.commit().await,
            #[cfg(feature = "postgres")]
            Self::Postgres(tx) => tx.commit().await,
        }
    }
}

impl From<DbBlob> for MockBlob {
    fn from(value: DbBlob) -> Self {
        let address = MockAddress::try_from(value.sender.as_slice())
            .expect("Malformed sender stored in database");
        let hash: [u8; 32] = value
            .hash
            .try_into()
            .expect("Blob hash should be 32 bytes long");
        MockBlob::new(value.data, address, hash)
    }
}

pub(crate) async fn connect(connection_string: &str) -> anyhow::Result<DbPool> {
    if connection_string.starts_with("sqlite:") {
        let options = SqliteConnectOptions::from_str(connection_string)?
            .log_statements(tracing::log::LevelFilter::Trace);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;
        return Ok(DbPool::Sqlite(pool));
    }

    #[cfg(feature = "postgres")]
    if connection_string.starts_with("postgres://")
        || connection_string.starts_with("postgresql://")
    {
        let options = PgConnectOptions::from_str(connection_string)?
            .log_statements(tracing::log::LevelFilter::Trace);
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;
        return Ok(DbPool::Postgres(pool));
    }

    #[cfg(not(feature = "postgres"))]
    if connection_string.starts_with("postgres://")
        || connection_string.starts_with("postgresql://")
    {
        anyhow::bail!("PostgreSQL support for mock-da requires the `postgres` feature");
    }

    anyhow::bail!(
        "Unsupported mock-da connection string `{connection_string}`. Expected sqlite:* or postgres:// / postgresql://"
    );
}

pub(crate) async fn setup_db(pool: &DbPool) -> anyhow::Result<()> {
    match pool {
        DbPool::Sqlite(pool) => sqlite::setup_db(pool).await,
        #[cfg(feature = "postgres")]
        DbPool::Postgres(pool) => postgres::setup_db(pool).await,
    }
}

pub(crate) async fn query_last_saved_block(pool: &DbPool) -> anyhow::Result<MockBlockHeader> {
    match pool {
        DbPool::Sqlite(pool) => sqlite::query_last_saved_block(pool).await,
        #[cfg(feature = "postgres")]
        DbPool::Postgres(pool) => postgres::query_last_saved_block(pool).await,
    }
}

pub(crate) async fn query_last_finalized_height(pool: &DbPool) -> anyhow::Result<u32> {
    match pool {
        DbPool::Sqlite(pool) => sqlite::query_last_finalized_height(pool).await,
        #[cfg(feature = "postgres")]
        DbPool::Postgres(pool) => postgres::query_last_finalized_height(pool).await,
    }
}

pub(crate) async fn get_block_header_at(
    pool: &DbPool,
    height: u32,
) -> anyhow::Result<Option<MockBlockHeader>> {
    match pool {
        DbPool::Sqlite(pool) => sqlite::get_block_header_at(pool, height).await,
        #[cfg(feature = "postgres")]
        DbPool::Postgres(pool) => postgres::get_block_header_at(pool, height).await,
    }
}

pub(crate) async fn get_block_header_at_tx(
    tx: &mut DbTx<'_>,
    height: u32,
) -> anyhow::Result<Option<MockBlockHeader>> {
    match tx {
        DbTx::Sqlite(tx) => sqlite::get_block_header_at(&mut **tx, height).await,
        #[cfg(feature = "postgres")]
        DbTx::Postgres(tx) => postgres::get_block_header_at(&mut **tx, height).await,
    }
}

pub(crate) async fn list_blob_hashes_at_tx(
    tx: &mut DbTx<'_>,
    height: u32,
) -> anyhow::Result<Vec<BlobHashData>> {
    match tx {
        DbTx::Sqlite(tx) => sqlite::list_blob_hashes_at(&mut **tx, height).await,
        #[cfg(feature = "postgres")]
        DbTx::Postgres(tx) => postgres::list_blob_hashes_at(&mut **tx, height).await,
    }
}

pub(crate) async fn insert_block_header_tx(
    tx: &mut DbTx<'_>,
    header: &MockBlockHeader,
) -> anyhow::Result<()> {
    match tx {
        DbTx::Sqlite(tx) => sqlite::insert_block_header(&mut **tx, header).await,
        #[cfg(feature = "postgres")]
        DbTx::Postgres(tx) => postgres::insert_block_header(&mut **tx, header).await,
    }
}

pub(crate) async fn upsert_finalized_height_tx(
    tx: &mut DbTx<'_>,
    height: u32,
) -> anyhow::Result<()> {
    match tx {
        DbTx::Sqlite(tx) => sqlite::upsert_finalized_height(&mut **tx, height).await,
        #[cfg(feature = "postgres")]
        DbTx::Postgres(tx) => postgres::upsert_finalized_height(&mut **tx, height).await,
    }
}

pub(crate) async fn insert_blob(pool: &DbPool, blob: &NewBlob) -> anyhow::Result<()> {
    match pool {
        DbPool::Sqlite(pool) => sqlite::insert_blob(pool, blob).await,
        #[cfg(feature = "postgres")]
        DbPool::Postgres(pool) => postgres::insert_blob(pool, blob).await,
    }
}

pub(crate) async fn list_blobs_at(pool: &DbPool, height: u32) -> anyhow::Result<Vec<DbBlob>> {
    match pool {
        DbPool::Sqlite(pool) => sqlite::list_blobs_at(pool, height).await,
        #[cfg(feature = "postgres")]
        DbPool::Postgres(pool) => postgres::list_blobs_at(pool, height).await,
    }
}

pub(crate) async fn delete_blobs_above_tx(tx: &mut DbTx<'_>, height: u32) -> anyhow::Result<()> {
    match tx {
        DbTx::Sqlite(tx) => sqlite::delete_blobs_above(&mut **tx, height).await,
        #[cfg(feature = "postgres")]
        DbTx::Postgres(tx) => postgres::delete_blobs_above(&mut **tx, height).await,
    }
}

pub(crate) async fn delete_block_headers_above_tx(
    tx: &mut DbTx<'_>,
    height: u32,
) -> anyhow::Result<()> {
    match tx {
        DbTx::Sqlite(tx) => sqlite::delete_block_headers_above(&mut **tx, height).await,
        #[cfg(feature = "postgres")]
        DbTx::Postgres(tx) => postgres::delete_block_headers_above(&mut **tx, height).await,
    }
}

pub(crate) async fn list_non_finalized_blob_hashes(
    pool: &DbPool,
    last_finalized_height: u32,
) -> anyhow::Result<Vec<BlobHashData>> {
    match pool {
        DbPool::Sqlite(pool) => {
            sqlite::list_non_finalized_blob_hashes(pool, last_finalized_height).await
        }
        #[cfg(feature = "postgres")]
        DbPool::Postgres(pool) => {
            postgres::list_non_finalized_blob_hashes(pool, last_finalized_height).await
        }
    }
}

pub(crate) async fn list_non_finalized_block_headers(
    pool: &DbPool,
    last_finalized_height: u32,
) -> anyhow::Result<Vec<DbBlockHeader>> {
    match pool {
        DbPool::Sqlite(pool) => {
            sqlite::list_non_finalized_block_headers(pool, last_finalized_height).await
        }
        #[cfg(feature = "postgres")]
        DbPool::Postgres(pool) => {
            postgres::list_non_finalized_block_headers(pool, last_finalized_height).await
        }
    }
}

pub(crate) async fn delete_blobs_by_ids(pool: &DbPool, ids: &[i32]) -> anyhow::Result<()> {
    match pool {
        DbPool::Sqlite(pool) => sqlite::delete_blobs_by_ids(pool, ids).await,
        #[cfg(feature = "postgres")]
        DbPool::Postgres(pool) => postgres::delete_blobs_by_ids(pool, ids).await,
    }
}

pub(crate) async fn move_blobs_to_height(
    pool: &DbPool,
    ids: &[i32],
    height: u32,
) -> anyhow::Result<()> {
    match pool {
        DbPool::Sqlite(pool) => sqlite::move_blobs_to_height(pool, ids, height).await,
        #[cfg(feature = "postgres")]
        DbPool::Postgres(pool) => postgres::move_blobs_to_height(pool, ids, height).await,
    }
}

pub(crate) async fn update_block_header_hashes(
    pool: &DbPool,
    height: u32,
    hash: &[u8; 32],
    prev_hash: &[u8; 32],
) -> anyhow::Result<()> {
    match pool {
        DbPool::Sqlite(pool) => {
            sqlite::update_block_header_hashes(pool, height, hash, prev_hash).await
        }
        #[cfg(feature = "postgres")]
        DbPool::Postgres(pool) => {
            postgres::update_block_header_hashes(pool, height, hash, prev_hash).await
        }
    }
}

pub(crate) fn build_batch_blob(
    height: i32,
    data: &[u8],
    sender: &MockAddress,
) -> (NewBlob, MockHash) {
    build_blob(height, data, sender, BATCH_NAMESPACE.to_string())
}

pub(crate) fn build_proof_blob(
    height: i32,
    data: &[u8],
    sender: &MockAddress,
) -> (NewBlob, MockHash) {
    build_blob(height, data, sender, PROOF_NAMESPACE.to_string())
}

fn build_blob(
    height: i32,
    data: &[u8],
    sender: &MockAddress,
    namespace: String,
) -> (NewBlob, MockHash) {
    let blob_hash = hash_to_array(data);
    (
        NewBlob {
            block_height: height,
            hash: blob_hash.to_vec(),
            data: data.to_vec(),
            namespace,
            sender: sender.as_ref().to_vec(),
        },
        MockHash(blob_hash),
    )
}

fn to_db_height(height: u32) -> anyhow::Result<i32> {
    i32::try_from(height).context("block height exceeds i32::MAX")
}

fn decode_block_header(
    height: i32,
    prev_hash: Vec<u8>,
    hash: Vec<u8>,
    created_at: DateTime<Utc>,
) -> anyhow::Result<MockBlockHeader> {
    let hash = MockHash::try_from(hash).map_err(|_| anyhow!("Corrupted `hash` in database"))?;
    let prev_hash =
        MockHash::try_from(prev_hash).map_err(|_| anyhow!("Corrupted `prev_hash` in database"))?;
    let time = Time::from_millis(created_at.timestamp_millis());

    Ok(MockBlockHeader {
        prev_hash,
        hash,
        height: height as u64,
        time,
    })
}

mod sqlite {
    use super::*;

    pub(super) async fn setup_db(pool: &SqlitePool) -> anyhow::Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS blobs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                block_height INTEGER NOT NULL,
                hash BLOB NOT NULL,
                data BLOB NOT NULL,
                namespace TEXT NOT NULL,
                sender BLOB NOT NULL
            )",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS block_headers (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                height INTEGER NOT NULL UNIQUE,
                prev_hash BLOB NOT NULL,
                hash BLOB NOT NULL,
                created_at TEXT NOT NULL
            )",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS last_finalized_height (
                id INTEGER PRIMARY KEY,
                value INTEGER NOT NULL
            )",
        )
        .execute(pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_blobs_block_height ON blobs (block_height)")
            .execute(pool)
            .await?;
        sqlx::query("PRAGMA journal_mode = WAL")
            .execute(pool)
            .await?;
        sqlx::query("PRAGMA busy_timeout = 5000")
            .execute(pool)
            .await?;
        Ok(())
    }

    pub(super) async fn query_last_saved_block<'e, E>(
        executor: E,
    ) -> anyhow::Result<MockBlockHeader>
    where
        E: Executor<'e, Database = Sqlite>,
    {
        let row = sqlx::query_as::<_, (i32, Vec<u8>, Vec<u8>, DateTime<Utc>)>(
            "SELECT height, prev_hash, hash, created_at
             FROM block_headers
             ORDER BY height DESC
             LIMIT 1",
        )
        .fetch_optional(executor)
        .await?;

        Ok(match row {
            Some((height, prev_hash, hash, created_at)) => {
                decode_block_header(height, prev_hash, hash, created_at)?
            }
            None => GENESIS_HEADER,
        })
    }

    pub(super) async fn query_last_finalized_height<'e, E>(executor: E) -> anyhow::Result<u32>
    where
        E: Executor<'e, Database = Sqlite>,
    {
        let value =
            sqlx::query_scalar::<_, i32>("SELECT value FROM last_finalized_height WHERE id = ?")
                .bind(FINALIZED_HEIGHT_ID)
                .fetch_optional(executor)
                .await?;

        Ok(value.unwrap_or_default() as u32)
    }

    pub(super) async fn get_block_header_at<'e, E>(
        executor: E,
        height: u32,
    ) -> anyhow::Result<Option<MockBlockHeader>>
    where
        E: Executor<'e, Database = Sqlite>,
    {
        let row = sqlx::query_as::<_, (i32, Vec<u8>, Vec<u8>, DateTime<Utc>)>(
            "SELECT height, prev_hash, hash, created_at
             FROM block_headers
             WHERE height = ?",
        )
        .bind(to_db_height(height)?)
        .fetch_optional(executor)
        .await?;

        row.map(|(height, prev_hash, hash, created_at)| {
            decode_block_header(height, prev_hash, hash, created_at)
        })
        .transpose()
    }

    pub(super) async fn list_blob_hashes_at<'e, E>(
        executor: E,
        height: u32,
    ) -> anyhow::Result<Vec<BlobHashData>>
    where
        E: Executor<'e, Database = Sqlite>,
    {
        let rows = sqlx::query_as::<_, (i32, Vec<u8>, Vec<u8>, String)>(
            "SELECT id, hash, sender, namespace
             FROM blobs
             WHERE block_height = ?
             ORDER BY id ASC",
        )
        .bind(to_db_height(height)?)
        .fetch_all(executor)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(id, hash, sender, namespace)| BlobHashData {
                id,
                hash,
                sender,
                namespace,
            })
            .collect())
    }

    pub(super) async fn insert_block_header<'e, E>(
        executor: E,
        header: &MockBlockHeader,
    ) -> anyhow::Result<()>
    where
        E: Executor<'e, Database = Sqlite>,
    {
        let created_at = DateTime::from_timestamp_millis(header.time.as_millis())
            .expect("valid block timestamp");
        sqlx::query(
            "INSERT INTO block_headers (height, prev_hash, hash, created_at)
             VALUES (?, ?, ?, ?)",
        )
        .bind(i32::try_from(header.height).context("block height exceeds i32::MAX")?)
        .bind(header.prev_hash.0.to_vec())
        .bind(header.hash.0.to_vec())
        .bind(created_at)
        .execute(executor)
        .await?;
        Ok(())
    }

    pub(super) async fn upsert_finalized_height<'e, E>(
        executor: E,
        height: u32,
    ) -> anyhow::Result<()>
    where
        E: Executor<'e, Database = Sqlite>,
    {
        sqlx::query(
            "INSERT INTO last_finalized_height (id, value)
             VALUES (?, ?)
             ON CONFLICT(id) DO UPDATE SET value = excluded.value",
        )
        .bind(FINALIZED_HEIGHT_ID)
        .bind(to_db_height(height)?)
        .execute(executor)
        .await?;
        Ok(())
    }

    pub(super) async fn insert_blob<'e, E>(executor: E, blob: &NewBlob) -> anyhow::Result<()>
    where
        E: Executor<'e, Database = Sqlite>,
    {
        sqlx::query(
            "INSERT INTO blobs (block_height, hash, data, namespace, sender)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(blob.block_height)
        .bind(&blob.hash)
        .bind(&blob.data)
        .bind(&blob.namespace)
        .bind(&blob.sender)
        .execute(executor)
        .await?;
        Ok(())
    }

    pub(super) async fn list_blobs_at<'e, E>(
        executor: E,
        height: u32,
    ) -> anyhow::Result<Vec<DbBlob>>
    where
        E: Executor<'e, Database = Sqlite>,
    {
        let rows = sqlx::query_as::<_, (Vec<u8>, Vec<u8>, String, Vec<u8>)>(
            "SELECT hash, data, namespace, sender
             FROM blobs
             WHERE block_height = ?
             ORDER BY id ASC",
        )
        .bind(to_db_height(height)?)
        .fetch_all(executor)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(hash, data, namespace, sender)| DbBlob {
                hash,
                data,
                namespace,
                sender,
            })
            .collect())
    }

    pub(super) async fn delete_blobs_above<'e, E>(executor: E, height: u32) -> anyhow::Result<()>
    where
        E: Executor<'e, Database = Sqlite>,
    {
        sqlx::query("DELETE FROM blobs WHERE block_height > ?")
            .bind(to_db_height(height)?)
            .execute(executor)
            .await?;
        Ok(())
    }

    pub(super) async fn delete_block_headers_above<'e, E>(
        executor: E,
        height: u32,
    ) -> anyhow::Result<()>
    where
        E: Executor<'e, Database = Sqlite>,
    {
        sqlx::query("DELETE FROM block_headers WHERE height > ?")
            .bind(to_db_height(height)?)
            .execute(executor)
            .await?;
        Ok(())
    }

    pub(super) async fn list_non_finalized_blob_hashes<'e, E>(
        executor: E,
        last_finalized_height: u32,
    ) -> anyhow::Result<Vec<BlobHashData>>
    where
        E: Executor<'e, Database = Sqlite>,
    {
        let rows = sqlx::query_as::<_, (i32, Vec<u8>, Vec<u8>, String)>(
            "SELECT id, hash, sender, namespace
             FROM blobs
             WHERE block_height > ?
             ORDER BY id ASC",
        )
        .bind(to_db_height(last_finalized_height)?)
        .fetch_all(executor)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(id, hash, sender, namespace)| BlobHashData {
                id,
                hash,
                sender,
                namespace,
            })
            .collect())
    }

    pub(super) async fn list_non_finalized_block_headers<'e, E>(
        executor: E,
        last_finalized_height: u32,
    ) -> anyhow::Result<Vec<DbBlockHeader>>
    where
        E: Executor<'e, Database = Sqlite>,
    {
        let rows = sqlx::query_as::<_, (i32, Vec<u8>, Vec<u8>)>(
            "SELECT height, prev_hash, hash
             FROM block_headers
             WHERE height > ?
             ORDER BY height ASC",
        )
        .bind(to_db_height(last_finalized_height)?)
        .fetch_all(executor)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(height, prev_hash, hash)| DbBlockHeader {
                height,
                prev_hash,
                hash,
            })
            .collect())
    }

    pub(super) async fn delete_blobs_by_ids<'e, E>(executor: E, ids: &[i32]) -> anyhow::Result<()>
    where
        E: Executor<'e, Database = Sqlite>,
    {
        if ids.is_empty() {
            return Ok(());
        }

        let mut query = QueryBuilder::<Sqlite>::new("DELETE FROM blobs WHERE id IN (");
        let mut separated = query.separated(", ");
        for id in ids {
            separated.push_bind(*id);
        }
        separated.push_unseparated(")");
        query.build().execute(executor).await?;
        Ok(())
    }

    pub(super) async fn move_blobs_to_height<'e, E>(
        executor: E,
        ids: &[i32],
        height: u32,
    ) -> anyhow::Result<()>
    where
        E: Executor<'e, Database = Sqlite>,
    {
        if ids.is_empty() {
            return Ok(());
        }

        let mut query = QueryBuilder::<Sqlite>::new("UPDATE blobs SET block_height = ");
        query.push_bind(to_db_height(height)?);
        query.push(" WHERE id IN (");
        let mut separated = query.separated(", ");
        for id in ids {
            separated.push_bind(*id);
        }
        separated.push_unseparated(")");
        query.build().execute(executor).await?;
        Ok(())
    }

    pub(super) async fn update_block_header_hashes<'e, E>(
        executor: E,
        height: u32,
        hash: &[u8; 32],
        prev_hash: &[u8; 32],
    ) -> anyhow::Result<()>
    where
        E: Executor<'e, Database = Sqlite>,
    {
        sqlx::query(
            "UPDATE block_headers
             SET hash = ?, prev_hash = ?
             WHERE height = ?",
        )
        .bind(hash.to_vec())
        .bind(prev_hash.to_vec())
        .bind(to_db_height(height)?)
        .execute(executor)
        .await?;
        Ok(())
    }
}

#[cfg(feature = "postgres")]
mod postgres {
    use super::*;

    pub(super) async fn setup_db(pool: &PgPool) -> anyhow::Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS blobs (
                id INTEGER GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,
                block_height INTEGER NOT NULL,
                hash BYTEA NOT NULL,
                data BYTEA NOT NULL,
                namespace TEXT NOT NULL,
                sender BYTEA NOT NULL
            )",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS block_headers (
                id INTEGER GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,
                height INTEGER NOT NULL UNIQUE,
                prev_hash BYTEA NOT NULL,
                hash BYTEA NOT NULL,
                created_at TIMESTAMPTZ NOT NULL
            )",
        )
        .execute(pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS last_finalized_height (
                id INTEGER PRIMARY KEY,
                value INTEGER NOT NULL
            )",
        )
        .execute(pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_blobs_block_height ON blobs (block_height)")
            .execute(pool)
            .await?;
        Ok(())
    }

    pub(super) async fn query_last_saved_block<'e, E>(
        executor: E,
    ) -> anyhow::Result<MockBlockHeader>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let row = sqlx::query_as::<_, (i32, Vec<u8>, Vec<u8>, DateTime<Utc>)>(
            "SELECT height, prev_hash, hash, created_at
             FROM block_headers
             ORDER BY height DESC
             LIMIT 1",
        )
        .fetch_optional(executor)
        .await?;

        Ok(match row {
            Some((height, prev_hash, hash, created_at)) => {
                decode_block_header(height, prev_hash, hash, created_at)?
            }
            None => GENESIS_HEADER,
        })
    }

    pub(super) async fn query_last_finalized_height<'e, E>(executor: E) -> anyhow::Result<u32>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let value =
            sqlx::query_scalar::<_, i32>("SELECT value FROM last_finalized_height WHERE id = $1")
                .bind(FINALIZED_HEIGHT_ID)
                .fetch_optional(executor)
                .await?;

        Ok(value.unwrap_or_default() as u32)
    }

    pub(super) async fn get_block_header_at<'e, E>(
        executor: E,
        height: u32,
    ) -> anyhow::Result<Option<MockBlockHeader>>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let row = sqlx::query_as::<_, (i32, Vec<u8>, Vec<u8>, DateTime<Utc>)>(
            "SELECT height, prev_hash, hash, created_at
             FROM block_headers
             WHERE height = $1",
        )
        .bind(to_db_height(height)?)
        .fetch_optional(executor)
        .await?;

        row.map(|(height, prev_hash, hash, created_at)| {
            decode_block_header(height, prev_hash, hash, created_at)
        })
        .transpose()
    }

    pub(super) async fn list_blob_hashes_at<'e, E>(
        executor: E,
        height: u32,
    ) -> anyhow::Result<Vec<BlobHashData>>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let rows = sqlx::query_as::<_, (i32, Vec<u8>, Vec<u8>, String)>(
            "SELECT id, hash, sender, namespace
             FROM blobs
             WHERE block_height = $1
             ORDER BY id ASC",
        )
        .bind(to_db_height(height)?)
        .fetch_all(executor)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(id, hash, sender, namespace)| BlobHashData {
                id,
                hash,
                sender,
                namespace,
            })
            .collect())
    }

    pub(super) async fn insert_block_header<'e, E>(
        executor: E,
        header: &MockBlockHeader,
    ) -> anyhow::Result<()>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let created_at = DateTime::from_timestamp_millis(header.time.as_millis())
            .expect("valid block timestamp");
        sqlx::query(
            "INSERT INTO block_headers (height, prev_hash, hash, created_at)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(i32::try_from(header.height).context("block height exceeds i32::MAX")?)
        .bind(header.prev_hash.0.to_vec())
        .bind(header.hash.0.to_vec())
        .bind(created_at)
        .execute(executor)
        .await?;
        Ok(())
    }

    pub(super) async fn upsert_finalized_height<'e, E>(
        executor: E,
        height: u32,
    ) -> anyhow::Result<()>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            "INSERT INTO last_finalized_height (id, value)
             VALUES ($1, $2)
             ON CONFLICT (id) DO UPDATE SET value = EXCLUDED.value",
        )
        .bind(FINALIZED_HEIGHT_ID)
        .bind(to_db_height(height)?)
        .execute(executor)
        .await?;
        Ok(())
    }

    pub(super) async fn insert_blob<'e, E>(executor: E, blob: &NewBlob) -> anyhow::Result<()>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            "INSERT INTO blobs (block_height, hash, data, namespace, sender)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(blob.block_height)
        .bind(&blob.hash)
        .bind(&blob.data)
        .bind(&blob.namespace)
        .bind(&blob.sender)
        .execute(executor)
        .await?;
        Ok(())
    }

    pub(super) async fn list_blobs_at<'e, E>(
        executor: E,
        height: u32,
    ) -> anyhow::Result<Vec<DbBlob>>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let rows = sqlx::query_as::<_, (Vec<u8>, Vec<u8>, String, Vec<u8>)>(
            "SELECT hash, data, namespace, sender
             FROM blobs
             WHERE block_height = $1
             ORDER BY id ASC",
        )
        .bind(to_db_height(height)?)
        .fetch_all(executor)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(hash, data, namespace, sender)| DbBlob {
                hash,
                data,
                namespace,
                sender,
            })
            .collect())
    }

    pub(super) async fn delete_blobs_above<'e, E>(executor: E, height: u32) -> anyhow::Result<()>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query("DELETE FROM blobs WHERE block_height > $1")
            .bind(to_db_height(height)?)
            .execute(executor)
            .await?;
        Ok(())
    }

    pub(super) async fn delete_block_headers_above<'e, E>(
        executor: E,
        height: u32,
    ) -> anyhow::Result<()>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query("DELETE FROM block_headers WHERE height > $1")
            .bind(to_db_height(height)?)
            .execute(executor)
            .await?;
        Ok(())
    }

    pub(super) async fn list_non_finalized_blob_hashes<'e, E>(
        executor: E,
        last_finalized_height: u32,
    ) -> anyhow::Result<Vec<BlobHashData>>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let rows = sqlx::query_as::<_, (i32, Vec<u8>, Vec<u8>, String)>(
            "SELECT id, hash, sender, namespace
             FROM blobs
             WHERE block_height > $1
             ORDER BY id ASC",
        )
        .bind(to_db_height(last_finalized_height)?)
        .fetch_all(executor)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(id, hash, sender, namespace)| BlobHashData {
                id,
                hash,
                sender,
                namespace,
            })
            .collect())
    }

    pub(super) async fn list_non_finalized_block_headers<'e, E>(
        executor: E,
        last_finalized_height: u32,
    ) -> anyhow::Result<Vec<DbBlockHeader>>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let rows = sqlx::query_as::<_, (i32, Vec<u8>, Vec<u8>)>(
            "SELECT height, prev_hash, hash
             FROM block_headers
             WHERE height > $1
             ORDER BY height ASC",
        )
        .bind(to_db_height(last_finalized_height)?)
        .fetch_all(executor)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(height, prev_hash, hash)| DbBlockHeader {
                height,
                prev_hash,
                hash,
            })
            .collect())
    }

    pub(super) async fn delete_blobs_by_ids<'e, E>(executor: E, ids: &[i32]) -> anyhow::Result<()>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if ids.is_empty() {
            return Ok(());
        }

        let mut query = QueryBuilder::<Postgres>::new("DELETE FROM blobs WHERE id IN (");
        let mut separated = query.separated(", ");
        for id in ids {
            separated.push_bind(*id);
        }
        separated.push_unseparated(")");
        query.build().execute(executor).await?;
        Ok(())
    }

    pub(super) async fn move_blobs_to_height<'e, E>(
        executor: E,
        ids: &[i32],
        height: u32,
    ) -> anyhow::Result<()>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if ids.is_empty() {
            return Ok(());
        }

        let mut query = QueryBuilder::<Postgres>::new("UPDATE blobs SET block_height = ");
        query.push_bind(to_db_height(height)?);
        query.push(" WHERE id IN (");
        let mut separated = query.separated(", ");
        for id in ids {
            separated.push_bind(*id);
        }
        separated.push_unseparated(")");
        query.build().execute(executor).await?;
        Ok(())
    }

    pub(super) async fn update_block_header_hashes<'e, E>(
        executor: E,
        height: u32,
        hash: &[u8; 32],
        prev_hash: &[u8; 32],
    ) -> anyhow::Result<()>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            "UPDATE block_headers
             SET hash = $1, prev_hash = $2
             WHERE height = $3",
        )
        .bind(hash.to_vec())
        .bind(prev_hash.to_vec())
        .bind(to_db_height(height)?)
        .execute(executor)
        .await?;
        Ok(())
    }
}
