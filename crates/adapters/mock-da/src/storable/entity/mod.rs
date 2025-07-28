//! [sea-orm](https://www.sea-ql.org/SeaORM/docs/index/) related code.
use sea_orm::sea_query::{Index, IndexCreateStatement};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait, QueryOrder, Schema};

use crate::config::GENESIS_HEADER;
use crate::MockBlockHeader;

pub mod blobs;
pub mod block_headers;
pub mod finalized_height;

pub(crate) const BATCH_NAMESPACE: &str = "batches";
pub(crate) const PROOF_NAMESPACE: &str = "proofs";

// DB Functions

pub(crate) async fn setup_db(db: &DatabaseConnection) -> anyhow::Result<()> {
    tracing::debug!("Setting up database");
    create_tables(db, blobs::Entity).await?;
    create_tables(db, block_headers::Entity).await?;
    create_tables(db, finalized_height::Entity).await?;
    let builder = db.get_database_backend();
    let index_stmt: IndexCreateStatement = Index::create()
        .name("idx-blobs-block_height")
        .table(blobs::Entity)
        .col(blobs::Column::BlockHeight)
        .if_not_exists()
        .to_owned();
    db.execute(builder.build(&index_stmt)).await?;
    if let DbBackend::Sqlite = db.get_database_backend() {
        db.execute(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "PRAGMA journal_mode = WAL".to_owned(),
        ))
        .await?;
    }
    Ok(())
}

pub(crate) async fn create_tables<E: EntityTrait>(
    db: &DatabaseConnection,
    entity: E,
) -> anyhow::Result<()> {
    let builder = db.get_database_backend();
    let schema = Schema::new(builder);
    db.execute(
        builder.build(
            &schema
                .create_table_from_entity(entity)
                .if_not_exists()
                .to_owned(),
        ),
    )
    .await?;
    Ok(())
}

pub(crate) async fn query_last_saved_block(
    db: &DatabaseConnection,
) -> anyhow::Result<MockBlockHeader> {
    let db_value = block_headers::Entity::find()
        .order_by_desc(block_headers::Column::Height)
        .one(db)
        .await?
        .map(MockBlockHeader::from);
    tracing::trace!(?db_value, "Loaded latest block header from database");
    Ok(db_value.unwrap_or(GENESIS_HEADER))
}

pub(crate) async fn query_last_finalized_height(db: &DatabaseConnection) -> anyhow::Result<u32> {
    let db_value = finalized_height::Entity::find_by_id(finalized_height::ID)
        .one(db)
        .await?
        .map(|model| model.value as u32);

    tracing::trace!(finalized_height = ?db_value, "Loaded latest finalized height from database");
    Ok(db_value.unwrap_or_default())
}
