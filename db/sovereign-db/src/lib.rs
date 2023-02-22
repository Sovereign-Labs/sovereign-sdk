use state_db::StateDB;

pub mod schema;
pub mod state_db;

pub struct SovereignDB {
    _state_db: StateDB,
}
