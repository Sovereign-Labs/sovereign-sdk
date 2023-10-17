use indoc::indoc;
use serde_json::Value;
use sqlx::{PgPool, Postgres, QueryBuilder};
use tracing::info;

use crate::api_v0::models::{self as m};
use crate::api_v0::{PageSelection, Pagination, Sorting, SortingOrder};
use crate::utils::HexString;

#[derive(Clone)]
pub struct Db<Pool = PgPool> {
    // Db connections and pools in SQLx use `Arc` internally, so they are
    // cheaply clonable.
    pub conn: Pool,
}

impl Db {
    pub async fn new(db_connection_url: &str) -> anyhow::Result<Self> {
        // TODO: obscure the connection URL in the log, as it may contain
        // sensitive information.
        info!(url = db_connection_url, "Connecting to database...");

        let db = Self {
            conn: PgPool::connect(db_connection_url).await?,
        };

        info!("Running migrations...");
        db.run_migrations().await?;

        info!("Database initialization successful.");

        Ok(db)
    }

    async fn run_migrations(&self) -> anyhow::Result<()> {
        sqlx::migrate!("./migrations").run(&self.conn).await?;
        Ok(())
    }

    pub async fn begin_transaction(
        &self,
    ) -> anyhow::Result<Db<sqlx::Transaction<'static, Postgres>>> {
        let transaction = self.conn.begin().await?;
        Ok(Db { conn: transaction })
    }
}

impl Db<sqlx::Transaction<'static, Postgres>> {
    pub async fn commit(self) -> anyhow::Result<()> {
        self.conn.commit().await?;
        Ok(())
    }

    pub async fn _rollback(self) -> anyhow::Result<()> {
        self.conn.rollback().await?;
        Ok(())
    }
}

/// Read operations.
impl Db {
    pub async fn get_tx_by_hash(&self, tx_hash: &HexString) -> anyhow::Result<Option<Value>> {
        let row_opt: Option<(Value,)> = sqlx::query_as(indoc!(
            r#"
            SELECT blob FROM transactions
            WHERE blob->>'tx_hash' = $1
            LIMIT 1
            "#
        ))
        .bind(tx_hash.to_string())
        .fetch_optional(&self.conn)
        .await?;

        Ok(row_opt.map(|r| r.0))
    }

    pub async fn chain_head(&self) -> anyhow::Result<Option<Value>> {
        // We always fetch the last known chain head value.
        let row: Option<(Value,)> =
            sqlx::query_as("SELECT chain_head_blob FROM indexing_status ORDER BY id DESC LIMIT 1")
                .fetch_optional(&self.conn)
                .await?;

        Ok(row.map(|r| r.0))
    }

    pub async fn get_block_by_hash(&self, hash: &HexString) -> anyhow::Result<Option<Value>> {
        let rows: Option<(Value,)> = sqlx::query_as(indoc!(
            r#"
            SELECT blob FROM blocks
            WHERE blob->>'hash' = $1
            "#
        ))
        .bind(hash.to_string())
        .fetch_optional(&self.conn)
        .await?;

        Ok(rows.map(|v| v.0))
    }

    pub async fn get_batch_by_hash(&self, hash: &HexString) -> anyhow::Result<Option<Value>> {
        let row_opt: Option<(Value,)> = sqlx::query_as(indoc!(
            r#"
            SELECT blob FROM batches
            WHERE blob->>'hash' = $1
            "#
        ))
        .bind(hash.to_string())
        .fetch_optional(&self.conn)
        .await?;

        Ok(row_opt.map(|r| r.0))
    }

    pub async fn get_events(&self, query: &m::EventsQuery) -> anyhow::Result<Vec<m::Event>> {
        let mut query_builder = SqlBuilder::new();
        query_builder
            .query
            .push("SELECT id, key, value FROM events");

        // Filtering
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

        let sorting = query.sort.map_to_string(|m::EventsQueryBy::Id| "id");

        query_builder.pagination(sorting.by, &query.pagination);
        query_builder.sorting(&sorting);

        let query = query_builder.query.build_query_as();
        Ok(query.fetch_all(&self.conn).await?)
    }

    pub async fn get_blocks(&self, query: &m::BlocksQuery) -> anyhow::Result<Vec<Value>> {
        let mut query_builder = SqlBuilder::new();
        query_builder.query.push("SELECT blob FROM blocks");

        // Filtering
        if let Some(filter) = &query.filter {
            match filter {
                m::BlocksQueryFilter::Hash(hash) => {
                    query_builder.push_condition("blob->>'hash' = ");
                    query_builder.query.push_bind(hash.to_string());
                }
                m::BlocksQueryFilter::Number(number) => {
                    query_builder.push_condition("blob->>'number' = ");
                    query_builder.query.push_bind(number.to_string());
                }
                m::BlocksQueryFilter::ParentHash(parent_hash) => {
                    query_builder.push_condition("blob->>'parentHash' = ");
                    query_builder.query.push_bind(parent_hash.to_string());
                }
            };
        }

        let sorting = query.sort.map_to_string(|by| match by {
            m::BlocksQuerySortBy::Number => "(blob->>'number')::bigint",
            m::BlocksQuerySortBy::Timestamp => "blob->>'timestamp'",
        });

        query_builder.pagination(sorting.by, &query.pagination);
        query_builder.sorting(&sorting);

        let query = query_builder.query.build_query_as();
        let rows: Vec<(Value,)> = query.fetch_all(&self.conn).await?;
        Ok(rows.into_iter().map(|v| v.0).collect())
    }

    pub async fn get_transactions(
        &self,
        query: &m::TransactionsQuery,
    ) -> anyhow::Result<Vec<Value>> {
        let mut query_builder = SqlBuilder::new();
        query_builder.query.push("SELECT blob FROM transactions");

        // Filtering
        if let Some(filter) = &query.filter {
            match filter {
                m::TransactionsQueryFilter::Batch(batch_id, _batch_txs_offset) => {
                    query_builder.push_condition("blob->>'batch_id' = ");
                    query_builder.query.push_bind(batch_id.to_string());
                }
                m::TransactionsQueryFilter::Hash(hash) => {
                    query_builder.push_condition("blob->>'tx_hash' = ");
                    query_builder.query.push_bind(hash.to_string());
                }
                m::TransactionsQueryFilter::Number(num) => {
                    query_builder.push_condition("blob->>'tx_number' = ");
                    query_builder.query.push_bind(num.to_string());
                }
            }
        }

        let sorting = query
            .sort
            .map_to_string(|m::TransactionsQuerySortBy::Id| "(blob->>'number')::bigint");

        query_builder.pagination(sorting.by, &query.pagination);
        query_builder.sorting(&sorting);

        let query = query_builder.query.build_query_as();
        let rows: Vec<(Value,)> = query.fetch_all(&self.conn).await?;
        Ok(rows.into_iter().map(|v| v.0).collect())
    }

    pub async fn get_batches(&self, query: &m::BatchesQuery) -> anyhow::Result<Vec<Value>> {
        let mut query_builder = SqlBuilder::new();
        query_builder.query.push("SELECT blob FROM batches");

        let sorting = query
            .sort
            .map_to_string(|m::BatchesQueryBy::Id| "(blob->>'number')::bigint");

        query_builder.pagination(sorting.by, &query.pagination);
        query_builder.sorting(&sorting);

        let query = query_builder.query.build_query_as();
        let rows: Vec<(Value,)> = query.fetch_all(&self.conn).await?;
        Ok(rows.into_iter().map(|v| v.0).collect())
    }
}

/// Write operations inside transactions.
impl Db<sqlx::Transaction<'static, Postgres>> {
    pub async fn insert_chain_head(&mut self, blob: &Value) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO indexing_status (chain_head_blob) VALUES ($1)")
            .bind(blob)
            .execute(&mut *self.conn)
            .await?;
        Ok(())
    }

    pub async fn upsert_blocks(&mut self, blocks: &[Value]) -> anyhow::Result<()> {
        if blocks.is_empty() {
            return Ok(());
        }

        let mut query = QueryBuilder::new("INSERT INTO blocks (blob) ");

        query.push_values(blocks, |mut builder, block| {
            builder.push_bind(block);
        });
        query.push(" ON CONFLICT ((blob->>'hash')) DO UPDATE SET blob = EXCLUDED.blob");

        query.build().execute(&mut *self.conn).await?;
        Ok(())
    }

    pub async fn upsert_transactions(&mut self, txs: &[Value]) -> anyhow::Result<()> {
        if txs.is_empty() {
            return Ok(());
        }

        let mut query = QueryBuilder::new("INSERT INTO transactions (blob) ");

        query.push_values(txs, |mut builder, tx| {
            builder.push_bind(tx);
        });
        query.push(" ON CONFLICT ((blob->>'hash')) DO UPDATE SET blob = EXCLUDED.blob");

        query.build().execute(&mut *self.conn).await?;
        Ok(())
    }

    pub async fn upsert_events(&mut self, events: &[m::Event]) -> anyhow::Result<()> {
        if events.is_empty() {
            return Ok(());
        }

        let mut query = QueryBuilder::new("INSERT INTO events (id, key, value) ");

        query.push_values(events, |mut builder, event| {
            builder.push_bind(event.id);
            builder.push_bind(&event.key);
            builder.push_bind(&event.value);
        });
        query.push(" ON CONFLICT ((id)) DO UPDATE SET value = EXCLUDED.value");

        query.build().execute(&mut *self.conn).await?;
        Ok(())
    }

    pub async fn upsert_batches(&mut self, batches: &[Value]) -> anyhow::Result<()> {
        if batches.is_empty() {
            return Ok(());
        }

        let mut query = QueryBuilder::new("INSERT INTO batches (blob) ");

        query.push_values(batches, |mut builder, blob| {
            builder.push_bind(blob);
        });
        query.push(" ON CONFLICT ((blob->>'hash')) DO NOTHING");

        query.build().execute(&mut *self.conn).await?;
        Ok(())
    }
}

/// A wrapper around [`sqlx::QueryBuilder`] which adds some custom functionality
/// on top of it:
///
/// - Syntactically correct `WHERE` clauses, with correct handling of `AND`s.
/// - Type-safe `ORDER BY` clauses.
/// - Cursor-based pagination.
struct SqlBuilder<'a> {
    query: QueryBuilder<'a, Postgres>,
    where_used_already: bool,
    pagination_done: bool,
    sorting_done: bool,
}

impl<'a> SqlBuilder<'a> {
    fn new() -> Self {
        Self {
            query: QueryBuilder::new("WITH subquery AS ("),
            where_used_already: false,
            pagination_done: false,
            sorting_done: false,
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

    fn pagination(&mut self, column_name: &str, pagination: &Pagination<i64>) {
        assert!(!self.pagination_done);
        self.pagination_done = true;

        let comparison_operator = match pagination.selection {
            PageSelection::Next => Some(">"),
            PageSelection::Prev => Some("<"),
            PageSelection::First | PageSelection::Last => None,
        };

        // If a cursor is not present, then next/prev effectively work as
        // first/last, which is fine.
        if let (Some(cmp_op), Some(cursor)) = (comparison_operator, pagination.cursor) {
            self.push_condition(&format!("{} {} ", column_name, cmp_op));
            self.query.push_bind(cursor);
        }

        self.query.push(" ORDER BY ");
        self.query.push(column_name);
        self.query.push(" ");
        self.query.push(match pagination.selection {
            PageSelection::Next | PageSelection::First => "ASC",
            PageSelection::Prev | PageSelection::Last => "DESC",
        });
        self.query.push(" LIMIT ");
        self.query.push(pagination.size.to_string());
    }

    /// MUST be called after all `WHERE` clauses have been pushed and after [`SqlBuilder::pagination`].
    fn sorting(&mut self, sorting: &Sorting<&str>) {
        assert!(self.pagination_done);
        assert!(!self.sorting_done);
        self.sorting_done = true;

        self.query.push(") SELECT * from subquery ORDER BY ");
        self.query.push(sorting.by);
        self.query.push(" ");
        self.query.push(match sorting.order {
            SortingOrder::Ascending => "ASC",
            SortingOrder::Descending => "DESC",
        });
    }
}
