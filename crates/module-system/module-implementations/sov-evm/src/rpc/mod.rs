pub use crate::primitive_types::MaybeSealedBlock;

mod block_resolution;
pub(crate) mod error;
pub(crate) mod handlers;
pub(crate) mod maybe_archival_state;
mod receipt_builder;
mod simulation;
mod synthetic_block_cache;

mod estimate_gas;
mod fee_history;
mod trace;

// Re-exports used by sibling submodules (handlers, estimate_gas, etc.)
pub(crate) use simulation::{
    apply_call_overrides, get_cfg_env_template, validate_call_fee_fields,
    validate_simulation_max_fee_against_base_fee,
};
