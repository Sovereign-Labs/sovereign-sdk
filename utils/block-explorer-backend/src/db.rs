use crate::models::EventsQuery;

use indoc::indoc;
use serde_json::Value;
use sqlx::{PgPool, QueryBuilder};
use tracing::info;

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

    pub async fn get_events(&self, query: &EventsQuery) -> anyhow::Result<Vec<Value>> {
        let mut where_clauses = vec![];
        let mut query_builder = QueryBuilder::new("SELECT blob FROM events");

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

        let query = query_builder.build();
        let events = query.fetch_all(&self.pool).await?;

        // FIXME: return the queried data.
        Ok(vec![])
    }
}

/// Write operations.
impl Db {
    pub async fn insert_tx(&self, tx: &Value) -> anyhow::Result<()> {
        sqlx::query(indoc!(
            r#"
			INSERT INTO transactions (blob)
			VALUES ($1)
			"#
        ))
        .bind(tx)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn upsert_block(&self, block: &Value) -> anyhow::Result<()> {
        sqlx::query(indoc!(
            r#"
			INSERT INTO blocks (blob)
			VALUES ($1)
			ON CONFLICT ((blob->>'hash')) DO UPDATE
			SET blob = EXCLUDED.blob
			"#
        ))
        .bind(block)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn insert_event(&self, event: &Value) -> anyhow::Result<()> {
        sqlx::query(indoc!(
            r#"
			INSERT INTO events (blob)
			VALUES ($1)
			"#
        ))
        .bind(event)
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}
