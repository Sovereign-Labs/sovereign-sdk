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
    Ok(())
}

pub(super) async fn query_last_saved_block<'e, E>(executor: E) -> anyhow::Result<MockBlockHeader>
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
        .ok_or_else(|| anyhow!("block timestamp out of valid DateTime range"))?;
    sqlx::query(
        "INSERT INTO block_headers (height, prev_hash, hash, created_at)
         VALUES (?, ?, ?, ?)",
    )
    .bind(i32::try_from(header.height).context("block height exceeds i32::MAX")?)
    .bind(header.prev_hash.0.as_slice())
    .bind(header.hash.0.as_slice())
    .bind(created_at)
    .execute(executor)
    .await?;
    Ok(())
}

pub(super) async fn upsert_finalized_height<'e, E>(executor: E, height: u32) -> anyhow::Result<()>
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
    .bind(blob.namespace)
    .bind(&blob.sender)
    .execute(executor)
    .await?;
    Ok(())
}

pub(super) async fn list_blobs_at<'e, E>(executor: E, height: u32) -> anyhow::Result<Vec<DbBlob>>
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
    .bind(hash.as_slice())
    .bind(prev_hash.as_slice())
    .bind(to_db_height(height)?)
    .execute(executor)
    .await?;
    Ok(())
}
