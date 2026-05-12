use std::str::FromStr;
use std::time::Duration;

use anyhow::{anyhow, Context};
use chrono::{DateTime, Utc};
use sov_rollup_interface::da::Time;
#[cfg(feature = "postgres")]
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
#[cfg(feature = "postgres")]
use sqlx::Postgres;
use sqlx::{ConnectOptions, Executor, QueryBuilder, Sqlite};

use crate::config::GENESIS_HEADER;
use crate::utils::hash_to_array;
use crate::{MockAddress, MockBlob, MockBlockHeader, MockHash};

#[cfg(feature = "postgres")]
mod postgres;
mod sqlite;

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
    namespace: &'static str,
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
        // Panics here indicate storage corruption: every row we wrote went through
        // typed inserts with fixed widths, so reading back malformed bytes means
        // the database file is no longer trustworthy.
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
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(30))
            .log_statements(tracing::log::LevelFilter::Trace);
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;
        return Ok(DbPool::Sqlite(pool));
    }

    if connection_string.starts_with("postgres://")
        || connection_string.starts_with("postgresql://")
    {
        #[cfg(feature = "postgres")]
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

pub(crate) fn build_blob(
    height: i32,
    data: &[u8],
    sender: &MockAddress,
    namespace: &'static str,
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
    let hash = MockHash::try_from(hash).context("Corrupted `hash` in database")?;
    let prev_hash = MockHash::try_from(prev_hash).context("Corrupted `prev_hash` in database")?;
    let time = Time::from_millis(created_at.timestamp_millis());

    Ok(MockBlockHeader {
        prev_hash,
        hash,
        height: height as u64,
        time,
    })
}
