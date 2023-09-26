use indoc::indoc;
use serde_json::Value;
use sqlx::{PgPool, QueryBuilder};
use tracing::info;

use crate::models as m;

#[derive(Clone)]
pub struct Db {
    // `PgPool` is an `Arc` internally, so it's cheaply clonable.
    pool: PgPool,
}

impl Db {
    pub async fn new(db_connection_url: &str) -> anyhow::Result<Self> {
        // TODO: obscure the connection URL in the log, as it may contain
        // sensitive information.
        info!(url = db_connection_url, "Connecting to database...");

        let db = Self {
            pool: PgPool::connect(&db_connection_url).await?,
        };

        info!("Running migrations...");
        db.run_migrations().await?;

        info!("Database initialization successful.");

        Ok(db)
    }

    async fn run_migrations(&self) -> anyhow::Result<()> {
        sqlx::migrate!("./migrations").run(&self.pool).await?;
        Ok(())
    }
}

/// Read operations.
impl Db {
    pub async fn get_tx_by_hash(&self, tx_hash: &[u8]) -> anyhow::Result<Option<Value>> {
        let row_opt: Option<(Value,)> = sqlx::query_as(indoc!(
            r#"
            SELECT blob FROM transactions
            WHERE blob->>'tx_hash' = $1
            LIMIT 1
            "#
        ))
        .bind(tx_hash)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row_opt.map(|r| r.0))
    }

    pub async fn get_blocks_by_height(&self, height: i64) -> anyhow::Result<Vec<Value>> {
        let rows: Vec<(Value,)> = sqlx::query_as(indoc!(
            r#"
            SELECT blob FROM blocks
            WHERE blob->>'height' = $1
            "#
        ))
        .bind(height)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|v| v.0).collect())
    }

    pub async fn get_events(&self, query: &m::EventsQuery) -> anyhow::Result<Vec<m::Event>> {
        let mut where_clauses = vec![];
        let mut query_builder = QueryBuilder::new("SELECT (id, key, value) FROM events");

        if let Some(event_id) = query.id {
            where_clauses.push("id = ?");
            query_builder.push_bind(event_id);
        }
        if let Some(tx_hash) = &query.tx_hash {
            where_clauses.push("tx_hash = ?");
            query_builder.push_bind(&tx_hash.0);
        }
        if let Some(tx_height) = query.tx_height {
            where_clauses.push("tx_height = ?");
            query_builder.push_bind(tx_height);
        }
        if let Some(key) = &query.key {
            where_clauses.push("key = ?");
            query_builder.push_bind(&key.0);
        }
        if let Some(offset) = query.offset {
            where_clauses.push("offset = ?");
            query_builder.push_bind(offset);
        }

        if !where_clauses.is_empty() {
            query_builder.push(" WHERE ");
            query_builder.push(where_clauses.join(" AND "));
        }

        let query = query_builder.build_query_as();
        Ok(query.fetch_all(&self.pool).await?)
    }
}

/// Write operations.
impl Db {
    pub async fn upsert_blocks(&self, blocks: &[&Value]) -> anyhow::Result<()> {
        if blocks.is_empty() {
            return Ok(());
        }

        let mut query = QueryBuilder::new("INSERT INTO blocks (blob) ");

        query.push_values(blocks, |mut builder, block| {
            builder.push_bind(block);
        });
        query.push(" ON CONFLICT ((blob->>'hash')) DO UPDATE SET blob = EXCLUDED.blob");

        query.build().execute(&self.pool).await?;
        Ok(())
    }

    pub async fn upsert_transactions(&self, txs: &[Value]) -> anyhow::Result<()> {
        if txs.is_empty() {
            return Ok(());
        }

        let mut query = QueryBuilder::new("INSERT INTO transactions (blob) ");

        query.push_values(txs, |mut builder, tx| {
            builder.push_bind(tx);
        });
        query.push(" ON CONFLICT ((blob->>'tx_hash')) DO UPDATE SET blob = EXCLUDED.blob");

        query.build().execute(&self.pool).await?;
        Ok(())
    }
}
