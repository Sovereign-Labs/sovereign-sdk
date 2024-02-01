pub mod tables;
/// Defines the on-disk representation of all types which are stored by the SDK in a format other than
/// their native format. Notable examples including slots, blocks, transactions and events, which
/// are split into their constituent parts and stored in separate tables for easy retrieval.
pub mod types;

pub use sov_schema_db::snapshot::{QueryManager, ReadOnlyDbSnapshot};
