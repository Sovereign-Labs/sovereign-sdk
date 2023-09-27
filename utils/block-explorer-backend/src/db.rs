use indoc::indoc;
use serde_json::Value;
use sqlx::{PgPool, Postgres, QueryBuilder};
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
            WHERE blob->>'number' = $1
            "#
        ))
        .bind(height)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|v| v.0).collect())
    }

    pub async fn get_events(&self, query: &m::EventsQuery) -> anyhow::Result<Vec<m::Event>> {
        let mut query_builder =
            WhereClausesBuilder::new(QueryBuilder::new("SELECT (id, key, value) FROM events"));

        if let Some(event_id) = query.id {
            query_builder.push_condition("id = ");
            query_builder.query.push_bind(event_id);
        }
        if let Some(tx_hash) = &query.tx_hash {
            query_builder.push_condition("tx_hash = ");
            query_builder.query.push_bind(&tx_hash.0);
        }
        if let Some(tx_height) = query.tx_height {
            query_builder.push_condition("tx_height = $?");
            query_builder.query.push_bind(tx_height);
        }
        if let Some(key) = &query.key {
            query_builder.push_condition("key = ");
            query_builder.query.push_bind(&key.0);
        }
        if let Some(offset) = query.offset {
            query_builder.push_condition("offset = ");
            query_builder.query.push_bind(offset);
        }

        let query = query_builder.query.build_query_as();
        Ok(query.fetch_all(&self.pool).await?)
    }

    pub async fn get_blocks(&self, query: &m::BlocksQuery) -> anyhow::Result<Vec<Value>> {
        let mut query_builder =
            WhereClausesBuilder::new(QueryBuilder::new("SELECT blob FROM blocks"));

        if let Some(hash) = &query.hash {
            query_builder.push_condition("blob->>'hash' = ");
            query_builder.query.push_bind(hash.to_string());
        }
        if let Some(height) = query.height {
            query_builder.push_condition("blob->>'number' = ");
            query_builder.query.push_bind(height.to_string());
        }
        if let Some(parent_hash) = &query.parent_hash {
            query_builder.push_condition("blob->>'parentHash' = ");
            query_builder.query.push_bind(parent_hash.to_string());
        }

        let query = query_builder.query.build_query_as();
        let rows: Vec<(Value,)> = query.fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(|v| v.0).collect())
    }
}

struct WhereClausesBuilder<'a> {
    query: QueryBuilder<'a, Postgres>,
    where_used_already: bool,
}

impl<'a> WhereClausesBuilder<'a> {
    fn new(query: QueryBuilder<'a, Postgres>) -> Self {
        Self {
            query,
            where_used_already: false,
        }
    }

    fn push_condition(&mut self, condition: &str) {
        if self.where_used_already {
            self.query.push(" AND ");
        } else {
            self.query.push(" WHERE ");
            self.where_used_already = true;
        }
        self.query.push(condition);
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

fn sort(query: &mut QueryBuilder<Postgres>, sorting: m::SortingQuery) {
    query.push(" ORDER BY ");
    query.push_bind(sorting.by);
    query.push(" ");
    query.push_bind(match sorting.direction {
        m::SortingQueryDirection::Ascending => "ASC",
        m::SortingQueryDirection::Descending => "DESC",
    });
}
