use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;
use sea_orm::Set;
use sov_rollup_interface::da::Time;

use crate::{MockBlockHeader, MockHash};

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "block_headers")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = true, column_type = "Integer")]
    /// Incremental block id.
    pub id: i32,
    /// Use i32 for compatibility with SQLite (no 64 bits ints by default)
    /// and PostgreSQL (index should be signed).
    #[sea_orm(unique)]
    pub height: i32,
    pub prev_hash: Vec<u8>,
    pub hash: Vec<u8>,
    #[sea_orm(column_type = "TimestampWithTimeZone")]
    pub created_at: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

impl From<Model> for MockBlockHeader {
    fn from(value: Model) -> Self {
        let hash = MockHash::try_from(value.hash).expect("Corrupted `hash` in database");
        let prev_hash =
            MockHash::try_from(value.prev_hash).expect("Corrupted `prev_hash` in database");
        let millis = value.created_at.timestamp_millis();
        let time = Time::from_millis(millis);

        MockBlockHeader {
            prev_hash,
            hash,
            height: value.height as u64,
            time,
        }
    }
}

impl From<MockBlockHeader> for ActiveModel {
    fn from(block_header: MockBlockHeader) -> Self {
        let timestamp = DateTime::from_timestamp_millis(block_header.time.as_millis()).unwrap();
        ActiveModel {
            height: Set(block_header.height as i32),
            prev_hash: Set(block_header.prev_hash.0.to_vec()),
            hash: Set(block_header.hash.0.to_vec()),
            created_at: Set(timestamp),
            ..Default::default()
        }
    }
}
