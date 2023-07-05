#![forbid(unsafe_code)]

use state_db::StateDB;

pub mod ledger_db;
pub mod rocks_db_config;
pub mod schema;
pub mod state_db;

pub struct SovereignDB {
    _state_db: StateDB,
}
